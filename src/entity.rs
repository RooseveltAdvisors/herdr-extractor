//! Built-in entity resolution for scrollback tokens.
//!
//! This module embeds a compact, dependency-free entity model:
//! 1. pattern NER for high-precision classes (email, IP, UUID, host, id, env)
//! 2. multi-token phrase chunking for named entities and tech compounds
//! 3. a linear feature scorer with baked-in weights for phrase worthiness and
//!    kind resolution when several classes compete
//!
//! No external runtime or model download is required. The scorer is small
//! enough to run on every extraction inside the picker open path.

use std::collections::HashSet;

use crate::extract::{ItemKind, NlpCandidate, MIN_LENGTH};

/// Baked-in linear weights for the phrase-worthiness scorer.
/// Features are intentionally simple and stable so the model stays inspectable.
#[derive(Clone, Copy, Debug)]
struct PhraseWeights {
    bias: f32,
    token_count: f32,
    avg_token_len: f32,
    title_case_ratio: f32,
    has_connector: f32,
    has_hyphen_or_underscore: f32,
    has_digit: f32,
    all_ascii_alpha: f32,
    starts_stopword: f32,
    looks_pathish: f32,
}

/// Default weights tuned offline against agent/terminal scrollback samples.
/// Positive score => keep as a phrase candidate.
const PHRASE_WEIGHTS: PhraseWeights = PhraseWeights {
    bias: -1.15,
    token_count: 0.55,
    avg_token_len: 0.12,
    title_case_ratio: 1.35,
    has_connector: 0.45,
    has_hyphen_or_underscore: 0.35,
    has_digit: -0.55,
    all_ascii_alpha: 0.25,
    starts_stopword: -1.40,
    looks_pathish: -2.20,
};

/// Kind-resolution weights when a span matches more than one coarse class.
/// Higher wins after adding class-specific evidence bonuses.
#[derive(Clone, Copy, Debug)]
struct KindPrior {
    kind: ItemKind,
    prior: f32,
}

const KIND_PRIORS: &[KindPrior] = &[
    KindPrior {
        kind: ItemKind::Url,
        prior: 0.95,
    },
    KindPrior {
        kind: ItemKind::Email,
        prior: 0.93,
    },
    KindPrior {
        kind: ItemKind::Uuid,
        prior: 0.92,
    },
    KindPrior {
        kind: ItemKind::Ip,
        prior: 0.90,
    },
    KindPrior {
        kind: ItemKind::Path,
        prior: 0.88,
    },
    KindPrior {
        kind: ItemKind::Host,
        prior: 0.84,
    },
    KindPrior {
        kind: ItemKind::Id,
        prior: 0.82,
    },
    KindPrior {
        kind: ItemKind::Env,
        prior: 0.80,
    },
    KindPrior {
        kind: ItemKind::Phrase,
        prior: 0.72,
    },
    KindPrior {
        kind: ItemKind::Code,
        prior: 0.65,
    },
    KindPrior {
        kind: ItemKind::Word,
        prior: 0.40,
    },
];

const CONNECTORS: &[&str] = &[
    "of", "and", "the", "for", "in", "on", "to", "de", "la", "van",
];
const PHRASE_STOP_START: &[&str] = &[
    "the", "a", "an", "and", "or", "but", "if", "then", "else", "when", "with", "from", "this",
    "that", "these", "those", "into", "onto", "over", "under", "after", "before", "while",
];

/// Run the built-in entity model over pane text and return scored candidates.
///
/// The caller merges these with regex candidates. This function is pure and
/// allocation-light relative to transformer NER.
pub fn resolve_entities(text: &str) -> Vec<NlpCandidate> {
    let mut out: Vec<NlpCandidate> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    for (span, kind, confidence) in pattern_ner(text) {
        push_unique(&mut out, &mut seen, span, kind, confidence);
    }
    for (span, confidence) in chunk_phrases(text) {
        let (kind, confidence) = resolve_kind(&span, confidence);
        push_unique(&mut out, &mut seen, span, kind, confidence);
    }
    out
}

fn push_unique(
    out: &mut Vec<NlpCandidate>,
    seen: &mut HashSet<String>,
    text: String,
    kind: ItemKind,
    confidence: f32,
) {
    if text.chars().count() < MIN_LENGTH || !confidence.is_finite() {
        return;
    }
    let key = text.to_ascii_lowercase();
    if !seen.insert(key) {
        // Prefer the higher-confidence (or higher-prior kind) entry.
        if let Some(existing) = out.iter_mut().find(|c| c.text.eq_ignore_ascii_case(&text)) {
            let existing_score = existing.confidence + kind_prior(existing.kind);
            let new_score = confidence + kind_prior(kind);
            if new_score > existing_score {
                existing.text = text;
                existing.kind = kind;
                existing.confidence = confidence;
            }
        }
        return;
    }
    out.push(NlpCandidate {
        text,
        kind,
        confidence,
    });
}

fn kind_prior(kind: ItemKind) -> f32 {
    KIND_PRIORS
        .iter()
        .find(|entry| entry.kind == kind)
        .map(|entry| entry.prior)
        .unwrap_or(0.3)
}

/// Pattern NER: high-precision single-span classes.
fn pattern_ner(text: &str) -> Vec<(String, ItemKind, f32)> {
    let mut hits = Vec::new();

    // Email
    for m in email_matches(text) {
        hits.push((m, ItemKind::Email, 0.97));
    }
    // UUID
    for m in uuid_matches(text) {
        hits.push((m, ItemKind::Uuid, 0.96));
    }
    // IPv4 / IPv6
    for m in ip_matches(text) {
        hits.push((m, ItemKind::Ip, 0.94));
    }
    // Host / FQDN / host:port
    for m in host_matches(text) {
        hits.push((m, ItemKind::Host, 0.88));
    }
    // Ticket / bead / issue ids
    for m in id_matches(text) {
        hits.push((m, ItemKind::Id, 0.90));
    }
    // Environment variables
    for m in env_matches(text) {
        hits.push((m, ItemKind::Env, 0.89));
    }
    // Markdown or bare URLs already handled by regex; still surface www. and bare domains here
    for m in bare_url_matches(text) {
        hits.push((m, ItemKind::Url, 0.86));
    }

    hits
}

fn email_matches(text: &str) -> Vec<String> {
    // Local-part @ domain with common TLD or multi-label host.
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'@' {
            let local = take_email_local(text, i);
            let domain = take_email_domain(text, i + 1);
            if let (Some(local), Some(domain)) = (local, domain) {
                let email = format!("{local}@{domain}");
                if email.chars().count() >= MIN_LENGTH {
                    out.push(email);
                }
            }
        }
        i += 1;
    }
    out
}

fn take_email_local(text: &str, at: usize) -> Option<String> {
    // `at` is a byte index of ASCII '@', so it is always a char boundary.
    let before = &text[..at];
    // rfind returns the *start* byte of the delimiter char. Advance by that
    // char's UTF-8 width so start never lands inside a multi-byte glyph
    // (e.g. curly quotes in scrollback), which would panic on slice.
    let start = before
        .char_indices()
        .rev()
        .find(|(_, c)| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '%' | '+' | '-')))
        .map(|(idx, c)| idx + c.len_utf8())
        .unwrap_or(0);
    let local = before[start..].trim_matches(|c: char| matches!(c, '.' | '-' | '_'));
    if local.is_empty() || local.len() > 64 {
        None
    } else {
        Some(local.to_string())
    }
}

fn take_email_domain(text: &str, start: usize) -> Option<String> {
    let rest = &text[start..];
    let end = rest
        .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '-')))
        .unwrap_or(rest.len());
    let domain = rest[..end].trim_matches(|c: char| matches!(c, '.' | '-'));
    if domain.contains('.') && domain.chars().any(|c| c.is_ascii_alphabetic()) {
        Some(domain.to_string())
    } else {
        None
    }
}

fn uuid_matches(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for token in tokenize_spans(text) {
        if is_uuid(&token) {
            out.push(token);
        }
    }
    out
}

fn is_uuid(value: &str) -> bool {
    let v = value.trim_matches(|c: char| matches!(c, '{' | '}' | '(' | ')'));
    let parts: Vec<&str> = v.split('-').collect();
    if parts.len() != 5 {
        return false;
    }
    let lens = [8, 4, 4, 4, 12];
    parts
        .iter()
        .zip(lens)
        .all(|(part, len)| part.len() == len && part.chars().all(|c| c.is_ascii_hexdigit()))
}

fn ip_matches(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for token in tokenize_spans(text) {
        let cleaned = token.trim_matches(|c: char| matches!(c, ',' | ';' | ')' | ']' | '>' | ':'));
        // host:port form handled by host; bare IPv4 here
        if is_ipv4(cleaned) || is_ipv6(cleaned) {
            out.push(cleaned.to_string());
        } else if let Some((host, port)) = cleaned.rsplit_once(':') {
            if is_ipv4(host) && port.chars().all(|c| c.is_ascii_digit()) && port.len() <= 5 {
                out.push(cleaned.to_string());
            }
        }
    }
    out
}

fn is_ipv4(value: &str) -> bool {
    let parts: Vec<&str> = value.split('.').collect();
    if parts.len() != 4 {
        return false;
    }
    parts.iter().all(|part| {
        if part.is_empty() || part.len() > 3 || !part.chars().all(|c| c.is_ascii_digit()) {
            return false;
        }
        part.parse::<u8>().is_ok()
    })
}

fn is_ipv6(value: &str) -> bool {
    if !value.contains(':') || value.chars().any(|c| c == '.') {
        // Skip IPv4-mapped forms for simplicity; rare in scrollback copy targets.
        if value.matches('.').count() == 3 && value.contains("::") {
            // still allow simple compressed forms without dotted tail elsewhere
        } else if value.contains('.') {
            return false;
        }
    }
    let v = value.trim_matches(|c: char| matches!(c, '[' | ']'));
    if !v.contains(':') {
        return false;
    }
    let ok_chars = v
        .chars()
        .all(|c| c.is_ascii_hexdigit() || c == ':' || c == '.');
    if !ok_chars {
        return false;
    }
    let colon_groups = v.split(':').count();
    (2..=8).contains(&colon_groups) && v.chars().filter(|c| *c == ':').count() >= 2
}

fn host_matches(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for token in tokenize_spans(text) {
        let cleaned = token.trim_matches(|c: char| {
            matches!(c, ',' | ';' | ')' | ']' | '>' | '"' | '\'' | '`' | '/')
        });
        if is_hostname(cleaned) || is_host_port(cleaned) {
            out.push(cleaned.to_string());
        }
    }
    out
}

fn is_host_port(value: &str) -> bool {
    let Some((host, port)) = value.rsplit_once(':') else {
        return false;
    };
    !port.is_empty()
        && port.len() <= 5
        && port.chars().all(|c| c.is_ascii_digit())
        && (is_hostname(host) || is_ipv4(host))
}

fn is_hostname(value: &str) -> bool {
    const BAD_TLDS: &[&str] = &[
        "gz", "tar", "tgz", "zip", "bz2", "xz", "log", "txt", "md", "rs", "go", "py", "js", "ts",
        "tsx", "jsx", "json", "toml", "yaml", "yml", "lock", "png", "jpg", "jpeg", "gif", "webp",
        "svg", "bin", "exe", "dll", "so", "dylib", "o", "a", "out", "class", "jar", "war", "rpm",
        "deb", "apk", "dmg", "iso", "img", "csv", "tsv", "sql", "sh", "bash", "zsh", "fish", "ps1",
        "bat", "cmd", "conf", "cfg", "ini", "env", "bak", "tmp", "swp", "swo", "orig", "rej",
        "git",
    ];
    if value.len() < 4 || value.contains('/') || value.contains('@') {
        return false;
    }
    // Require at least one dot and a plausible TLD, or .local/.internal
    let labels: Vec<&str> = value.split('.').collect();
    if labels.len() < 2 {
        return false;
    }
    let tld = labels.last().copied().unwrap_or("");
    let tld_l = tld.to_ascii_lowercase();
    if BAD_TLDS.contains(&tld_l.as_str()) {
        return false;
    }
    let tld_ok = (2..=24).contains(&tld.len())
        && tld.chars().all(|c| c.is_ascii_alphanumeric())
        && tld.chars().any(|c| c.is_ascii_alphabetic());
    if !tld_ok {
        return false;
    }
    labels.iter().all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && label
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-')
            && !label.starts_with('-')
            && !label.ends_with('-')
    }) && !value.starts_with('.')
        && !value.ends_with('.')
        // Avoid version-like 1.2.3 being classed as host unless TLD is alpha-heavy
        && labels
            .iter()
            .take(labels.len() - 1)
            .any(|label| label.chars().any(|c| c.is_ascii_alphabetic()))
}

fn id_matches(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for token in tokenize_spans(text) {
        let cleaned = token.trim_matches(|c: char| matches!(c, ',' | ';' | ')' | ']' | '>' | ':'));
        if is_ticket_id(cleaned) {
            out.push(cleaned.to_string());
        }
    }
    // Also catch "#1234" and "PR #88" style issue refs attached to words
    for (idx, ch) in text.char_indices() {
        if ch == '#' {
            let rest = &text[idx + ch.len_utf8()..];
            let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            // Ticket refs only: 2–5 digits. 6/8 digit runs are hex colors (#rrggbb).
            if (2..=5).contains(&digits.len()) {
                out.push(format!("#{digits}"));
            }
        }
    }
    out
}

fn is_ticket_id(value: &str) -> bool {
    // PROJECT-123, wiseman-8ure, bd-42, fm-abc12, JIRA_9
    if value.len() < 5 || value.len() > 48 || is_uuid(value) {
        return false;
    }
    let parts: Vec<&str> = if value.contains('-') {
        value.split('-').collect()
    } else if value.contains('_') {
        value.split('_').collect()
    } else {
        return false;
    };
    if parts.len() < 2 || parts.len() > 4 {
        return false;
    }
    // Reject UUID component pairs like e29b-41d4 or a716-446655440000.
    if parts
        .iter()
        .all(|p| p.chars().all(|c| c.is_ascii_hexdigit()))
    {
        return false;
    }
    let head = parts[0];
    let tail = parts[parts.len() - 1];
    let head_ok = (2..=24).contains(&head.len())
        && head.chars().all(|c| c.is_ascii_alphanumeric())
        && head.chars().any(|c| c.is_ascii_alphabetic());
    let tail_ok = !tail.is_empty()
        && tail.len() <= 16
        && tail.chars().all(|c| c.is_ascii_alphanumeric())
        && value.chars().any(|c| c.is_ascii_digit());
    head_ok && tail_ok && !is_ipv4(value) && !value.contains('.')
}

fn env_matches(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for token in tokenize_spans(text) {
        let cleaned = token
            .trim_matches(|c: char| matches!(c, ',' | ';' | ')' | ']' | '>' | '"' | '\'' | '`'));
        // FOO_BAR or $FOO_BAR or FOO_BAR=value (capture name)
        let name = cleaned
            .strip_prefix('$')
            .unwrap_or(cleaned)
            .split_once('=')
            .map(|(name, _)| name)
            .unwrap_or(cleaned.strip_prefix('$').unwrap_or(cleaned));
        if is_env_name(name) {
            out.push(if cleaned.starts_with('$') {
                format!("${name}")
            } else {
                name.to_string()
            });
        }
    }
    out
}

fn is_env_name(value: &str) -> bool {
    if value.len() < 5 || value.len() > 64 {
        return false;
    }
    let chars: Vec<char> = value.chars().collect();
    if !chars[0].is_ascii_uppercase() && chars[0] != '_' {
        return false;
    }
    let all_ok = chars
        .iter()
        .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || *c == '_');
    all_ok && value.contains('_') && value.chars().any(|c| c.is_ascii_alphabetic())
}

fn bare_url_matches(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for token in tokenize_spans(text) {
        let cleaned = token.trim_matches(|c: char| matches!(c, ',' | ';' | ')' | ']' | '>' | '.'));
        if let Some(url) = as_bare_url(cleaned) {
            out.push(url);
        }
    }
    out
}

fn as_bare_url(value: &str) -> Option<String> {
    let lower = value.to_ascii_lowercase();
    if lower.starts_with("www.") && is_hostname(value.trim_start_matches("www.").split('/').next()?)
    {
        return Some(format!("https://{value}"));
    }
    None
}

/// Multi-token phrase chunker with linear worthiness model.
fn chunk_phrases(text: &str) -> Vec<(String, f32)> {
    let mut out = Vec::new();
    for line in text.lines() {
        let tokens = line_tokens(line);
        if tokens.is_empty() {
            continue;
        }
        // Title-case runs and connector-joined names: "Jon Roosevelt", "Board of Directors"
        let mut i = 0;
        while i < tokens.len() {
            if !looks_title_token(tokens[i].1) {
                i += 1;
                continue;
            }
            let mut j = i + 1;
            let mut used_connector = false;
            while j < tokens.len() {
                let prev_end = tokens[j - 1].0 + tokens[j - 1].1.len();
                let gap = &line[prev_end..tokens[j].0];
                if gap.contains(',') || gap.contains(';') || gap.contains('|') {
                    break;
                }
                let tok = &tokens[j].1;
                if looks_title_token(tok) {
                    j += 1;
                    continue;
                }
                if CONNECTORS.contains(&tok.to_ascii_lowercase().as_str())
                    && j + 1 < tokens.len()
                    && looks_title_token(tokens[j + 1].1)
                {
                    used_connector = true;
                    j += 2;
                    continue;
                }
                break;
            }
            let count = j - i;
            if count >= 2 || (count == 1 && used_connector) {
                // Join token texts so commas/punctuation between title runs
                // do not get re-included into one mega-phrase.
                let phrase = tokens[i..j]
                    .iter()
                    .map(|(_, tok)| *tok)
                    .collect::<Vec<_>>()
                    .join(" ");
                let score = score_phrase(&phrase, used_connector);
                if score >= 0.55 {
                    out.push((phrase, score));
                }
            }
            i = j.max(i + 1);
        }

        // Tech compounds already single-token (kebab/snake) promoted to phrase-level entities
        for (_, tok) in &tokens {
            if is_tech_compound(tok) {
                let score = score_phrase(tok, false);
                if score >= 0.60 {
                    out.push(((*tok).to_string(), score));
                }
            }
        }
    }
    out
}

fn looks_title_token(tok: &str) -> bool {
    let mut chars = tok.chars();
    match chars.next() {
        Some(c) if c.is_uppercase() => {
            let rest_ok = chars.all(|c| c.is_alphabetic() || c == '\'' || c == '-');
            rest_ok && tok.chars().count() >= 2
        }
        _ => false,
    }
}

fn is_tech_compound(tok: &str) -> bool {
    let len = tok.chars().count();
    if !(5..=64).contains(&len) {
        return false;
    }
    let has_sep = tok.contains('-') || tok.contains('_') || tok.contains('.');
    let alnum_ish = tok
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    has_sep
        && alnum_ish
        && tok.chars().any(|c| c.is_ascii_alphabetic())
        && !tok.starts_with('-')
        && !tok.ends_with('-')
        && !is_ipv4(tok)
        && !is_uuid(tok)
}

fn score_phrase(phrase: &str, has_connector: bool) -> f32 {
    let tokens: Vec<&str> = phrase
        .split(|c: char| c.is_whitespace() || c == '-' || c == '_')
        .filter(|t| !t.is_empty())
        .collect();
    if tokens.is_empty() {
        return 0.0;
    }
    let token_count = tokens.len() as f32;
    let avg_token_len =
        tokens.iter().map(|t| t.chars().count()).sum::<usize>() as f32 / token_count;
    let title_case_ratio = tokens
        .iter()
        .filter(|t| looks_title_token(t) || CONNECTORS.contains(&t.to_ascii_lowercase().as_str()))
        .count() as f32
        / token_count;
    let has_hyphen_or_underscore = if phrase.contains('-') || phrase.contains('_') {
        1.0
    } else {
        0.0
    };
    let has_digit = if phrase.chars().any(|c| c.is_ascii_digit()) {
        1.0
    } else {
        0.0
    };
    let all_ascii_alpha = if phrase
        .chars()
        .all(|c| c.is_ascii_alphabetic() || c.is_whitespace() || matches!(c, '-' | '\'' | '_'))
    {
        1.0
    } else {
        0.0
    };
    let first = tokens[0].to_ascii_lowercase();
    let starts_stopword = if PHRASE_STOP_START.contains(&first.as_str()) {
        1.0
    } else {
        0.0
    };
    let looks_pathish = if phrase.contains('/') || phrase.starts_with('~') {
        1.0
    } else {
        0.0
    };

    let w = PHRASE_WEIGHTS;
    let logit = w.bias
        + w.token_count * token_count
        + w.avg_token_len * avg_token_len
        + w.title_case_ratio * title_case_ratio
        + w.has_connector * if has_connector { 1.0 } else { 0.0 }
        + w.has_hyphen_or_underscore * has_hyphen_or_underscore
        + w.has_digit * has_digit
        + w.all_ascii_alpha * all_ascii_alpha
        + w.starts_stopword * starts_stopword
        + w.looks_pathish * looks_pathish;

    sigmoid(logit)
}

fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

/// Resolve the best kind for a free-form span using priors + evidence.
fn resolve_kind(span: &str, base: f32) -> (ItemKind, f32) {
    let mut best = (ItemKind::Phrase, base * kind_prior(ItemKind::Phrase));
    let mut consider = |kind: ItemKind, evidence: f32| {
        let score = base * kind_prior(kind) + evidence;
        if score > best.1 {
            best = (kind, score);
        }
    };

    if span.contains('@') && email_matches(span).iter().any(|e| e == span) {
        consider(ItemKind::Email, 0.20);
    }
    if is_uuid(span) {
        consider(ItemKind::Uuid, 0.20);
    }
    if is_ipv4(span) || is_ipv6(span) {
        consider(ItemKind::Ip, 0.18);
    }
    if is_hostname(span) || is_host_port(span) {
        consider(ItemKind::Host, 0.15);
    }
    if is_ticket_id(span)
        || (span.starts_with('#') && span[1..].chars().all(|c| c.is_ascii_digit()))
    {
        consider(ItemKind::Id, 0.16);
    }
    if is_env_name(span.trim_start_matches('$')) {
        consider(ItemKind::Env, 0.14);
    }
    if span.contains('/') && !span.contains(' ') {
        consider(ItemKind::Path, 0.12);
    }
    if is_tech_compound(span) && span.chars().any(|c| c.is_ascii_lowercase()) {
        consider(ItemKind::Code, 0.08);
    }

    // Map score back into a confidence in [0.55, 0.99]
    let confidence = (best.1).clamp(0.55, 0.99);
    (best.0, confidence)
}

fn tokenize_spans(text: &str) -> Vec<String> {
    text.split(|c: char| {
        c.is_whitespace()
            || matches!(
                c,
                ',' | ';' | '(' | ')' | '[' | ']' | '{' | '}' | '<' | '>' | '|' | '"' | '\''
            )
    })
    .filter(|t| !t.is_empty())
    .map(|t| t.to_string())
    .collect()
}

fn line_tokens(line: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let mut start = None;
    for (idx, ch) in line.char_indices() {
        let is_break = ch.is_whitespace()
            || matches!(
                ch,
                ',' | ';'
                    | '('
                    | ')'
                    | '['
                    | ']'
                    | '{'
                    | '}'
                    | '<'
                    | '>'
                    | '|'
                    | '"'
                    | '\''
                    | '`'
                    | '='
            );
        if is_break {
            if let Some(s) = start.take() {
                out.push((s, &line[s..idx]));
            }
        } else if start.is_none() {
            start = Some(idx);
        }
    }
    if let Some(s) = start {
        out.push((s, &line[s..]));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_core_entity_classes() {
        let text = concat!(
            "ping 10.0.0.5:8080\n",
            "mail jon@arcs.health please\n",
            "id 550e8400-e29b-41d4-a716-446655440000\n",
            "host api.staging.example.internal ready\n",
            "bead wiseman-8ure claimed\n",
            "export HERDR_SOCKET_PATH=/tmp/x\n",
            "see www.example.com/docs\n",
            "talked to Jon Roosevelt about Covenant Clinics\n",
        );
        let found = resolve_entities(text);
        assert!(found
            .iter()
            .any(|c| c.kind == ItemKind::Ip && c.text.starts_with("10.0.0.5")));
        assert!(found
            .iter()
            .any(|c| c.kind == ItemKind::Email && c.text == "jon@arcs.health"));
        assert!(found.iter().any(|c| c.kind == ItemKind::Uuid));
        assert!(found
            .iter()
            .any(|c| c.kind == ItemKind::Host && c.text.contains("example.internal")));
        assert!(found
            .iter()
            .any(|c| c.kind == ItemKind::Id && c.text == "wiseman-8ure"));
        assert!(found.iter().any(|c| c.kind == ItemKind::Env
            && (c.text == "HERDR_SOCKET_PATH" || c.text == "$HERDR_SOCKET_PATH")));
        assert!(found.iter().any(|c| c.kind == ItemKind::Url));
        assert!(found.iter().any(|c| c.kind == ItemKind::Phrase
            && (c.text.contains("Jon Roosevelt") || c.text.contains("Covenant Clinics"))));
    }

    #[test]
    fn phrase_model_rejects_path_noise() {
        let score = score_phrase("/tmp/build/output.log", false);
        assert!(score < 0.55, "path scored {score}");
        let good = score_phrase("Covenant Clinics", false);
        assert!(good >= 0.55, "name scored {good}");
    }

    #[test]
    fn ticket_and_hash_refs() {
        let found = resolve_entities("fixes #1234 and PROJ-99 on branch");
        assert!(found.iter().any(|c| c.text == "#1234"));
        assert!(found.iter().any(|c| c.text == "PROJ-99"));
    }

    #[test]
    fn email_after_multibyte_quote_does_not_panic() {
        // Curly quotes are common in agent scrollback. The local-part scanner
        // must not slice inside the multi-byte glyph when backing up from '@'.
        let text = "said “hi jon@arcs.health please”\n";
        let found = resolve_entities(text);
        assert!(found
            .iter()
            .any(|c| c.kind == ItemKind::Email && c.text == "jon@arcs.health"));
    }
}
