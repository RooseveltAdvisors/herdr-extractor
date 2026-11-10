//! Pure scrollback token extraction.
//!
//! The extractor deliberately has no terminal or socket dependencies. It
//! removes terminal debris before matching, runs pattern filters for stable
//! semantic tokens, then applies the built-in entity resolver for names,
//! phrases, and high-precision classes the regex pass can miss.

use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;
use unicode_width::UnicodeWidthStr;

/// Semantic class of an extracted item. Stable keys are part of the sidecar
/// and UI contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ItemKind {
    Url,
    Path,
    Quote,
    SQuote,
    Word,
    Command,
    Hash,
    Version,
    Error,
    Code,
    Email,
    Ip,
    Uuid,
    Host,
    Id,
    Env,
    Phrase,
    Date,
    Color,
}

impl ItemKind {
    pub fn stable_key(self) -> &'static str {
        match self {
            Self::Url => "url",
            Self::Path => "path",
            Self::Quote => "quote",
            Self::SQuote => "squote",
            Self::Word => "word",
            Self::Command => "command",
            Self::Hash => "hash",
            Self::Version => "version",
            Self::Error => "error",
            Self::Code => "code",
            Self::Email => "email",
            Self::Ip => "ip",
            Self::Uuid => "uuid",
            Self::Host => "host",
            Self::Id => "id",
            Self::Env => "env",
            Self::Phrase => "phrase",
            Self::Date => "date",
            Self::Color => "color",
        }
    }

    pub fn from_stable_key(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "url" => Some(Self::Url),
            "path" => Some(Self::Path),
            "quote" => Some(Self::Quote),
            "squote" | "single-quote" => Some(Self::SQuote),
            "word" => Some(Self::Word),
            "command" => Some(Self::Command),
            "hash" => Some(Self::Hash),
            "version" => Some(Self::Version),
            "error" => Some(Self::Error),
            "code" => Some(Self::Code),
            "email" | "mail" => Some(Self::Email),
            "ip" | "ipv4" | "ipv6" => Some(Self::Ip),
            "uuid" | "guid" => Some(Self::Uuid),
            "host" | "hostname" | "fqdn" => Some(Self::Host),
            "id" | "ticket" | "issue" | "bead" => Some(Self::Id),
            "env" | "envvar" | "environment" => Some(Self::Env),
            "phrase" | "entity" | "name" => Some(Self::Phrase),
            "date" | "isodate" => Some(Self::Date),
            "color" | "hexcolor" => Some(Self::Color),
            _ => None,
        }
    }

    pub fn rank_value(self) -> i64 {
        match self {
            Self::Url | Self::Path | Self::Email | Self::Error => 7,
            Self::Ip | Self::Uuid | Self::Host | Self::Id => 6,
            Self::Command | Self::Hash | Self::Version | Self::Env | Self::Date => 5,
            Self::Color => 6,
            Self::Phrase | Self::Quote | Self::SQuote | Self::Code => 3,
            Self::Word => 1,
        }
    }

    /// Every stable kind the extractor and sidecar contract recognize.
    pub fn all() -> &'static [ItemKind] {
        &KIND_ALL
    }
}

const KIND_ALL: [ItemKind; 19] = [
    ItemKind::Url,
    ItemKind::Path,
    ItemKind::Quote,
    ItemKind::SQuote,
    ItemKind::Word,
    ItemKind::Command,
    ItemKind::Hash,
    ItemKind::Version,
    ItemKind::Error,
    ItemKind::Code,
    ItemKind::Email,
    ItemKind::Ip,
    ItemKind::Uuid,
    ItemKind::Host,
    ItemKind::Id,
    ItemKind::Env,
    ItemKind::Phrase,
    ItemKind::Date,
    ItemKind::Color,
];

/// One copy-eligible token from pane scrollback.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtractItem {
    pub text: String,
    pub kind: ItemKind,
}

/// A candidate returned by an optional NLP sidecar or the built-in resolver.
#[derive(Clone, Debug, PartialEq)]
pub struct NlpCandidate {
    pub text: String,
    pub kind: ItemKind,
    pub confidence: f32,
}

pub const MIN_LENGTH: usize = 5;

pub fn extract_items_from_visible_text(text: &str) -> Vec<ExtractItem> {
    extract_items_from_flat(text)
}

pub fn extract_items_from_visible_text_with_wrap_width(
    text: &str,
    wrap_width: Option<usize>,
) -> Vec<ExtractItem> {
    let Some(width) = wrap_width.filter(|width| *width > 0) else {
        return extract_items_from_flat(text);
    };
    let rows: Vec<&str> = text.split('\n').collect();
    let mut rejoined = String::with_capacity(text.len());
    for (index, row) in rows.iter().enumerate() {
        rejoined.push_str(row);
        if index + 1 < rows.len() && UnicodeWidthStr::width(*row) != width {
            rejoined.push('\n');
        }
    }
    extract_items_from_flat(&rejoined)
}

/// Merge structured sidecar/builtin candidates with regex candidates. Candidates
/// below the threshold are ignored; an accepted candidate replaces an item with
/// the same canonical text so NLP can improve its semantic class.
pub fn merge_nlp_candidates(
    regex_items: Vec<ExtractItem>,
    candidates: &[NlpCandidate],
    confidence_threshold: f32,
) -> Vec<ExtractItem> {
    let mut merged = regex_items;
    for candidate in candidates {
        if !candidate.confidence.is_finite()
            || candidate.confidence < confidence_threshold
            || candidate.text.chars().count() < MIN_LENGTH
        {
            continue;
        }
        let text = canonicalize(&candidate.text);
        let floor = min_len_for(candidate.kind, &text);
        if text.chars().count() < floor || junk_token(&text) {
            continue;
        }
        if let Some(existing) = merged.iter_mut().find(|item| item.text == text) {
            // Prefer the higher-ranked semantic class on a tie of identity.
            if candidate.kind.rank_value() >= existing.kind.rank_value() {
                existing.kind = candidate.kind;
            }
        } else {
            merged.push(ExtractItem {
                text,
                kind: candidate.kind,
            });
        }
    }
    let ranked = merged
        .iter()
        .map(|item| RankedItem {
            item: item.clone(),
            offset: 0,
        })
        .collect::<Vec<_>>();
    dedupe_and_rank(&ranked)
}

#[derive(Clone)]
struct RankedItem {
    item: ExtractItem,
    offset: usize,
}

fn extract_items_from_flat(text: &str) -> Vec<ExtractItem> {
    let clean = strip_terminal_artifacts(text);
    let mut candidates = Vec::new();
    candidates.extend(filter_urls(&clean));
    candidates.extend(filter_markdown_urls(&clean));
    candidates.extend(filter_emails(&clean));
    candidates.extend(filter_ips(&clean));
    candidates.extend(filter_uuids(&clean));
    candidates.extend(filter_hosts(&clean));
    candidates.extend(filter_paths(&clean));
    candidates.extend(filter_ids(&clean));
    candidates.extend(filter_envs(&clean));
    candidates.extend(filter_quotes(&clean));
    candidates.extend(filter_s_quotes(&clean));
    candidates.extend(filter_code(&clean));
    candidates.extend(filter_commands(&clean));
    candidates.extend(filter_hashes(&clean));
    candidates.extend(filter_hex_addrs(&clean));
    candidates.extend(filter_hex_colors(&clean));
    candidates.extend(filter_iso_dates(&clean));
    candidates.extend(filter_github_shorthand(&clean));
    candidates.extend(filter_versions(&clean));
    candidates.extend(filter_errors(&clean));
    candidates.extend(filter_phrases(&clean));

    let specialized: HashSet<String> = candidates
        .iter()
        .map(|candidate: &RankedItem| candidate.item.text.clone())
        .collect();
    for candidate in filter_words(&clean) {
        if !specialized.contains(&candidate.item.text)
            && !candidates.iter().any(|specialized| {
                word_redundant_with_specialized(&candidate.item.text, &specialized.item)
            })
        {
            candidates.push(candidate);
        }
    }

    // Built-in entity resolver always runs: it adds phrases/names the regex
    // pass misses and upgrades kind labels when confidence is high.
    let regex_only = dedupe_and_rank(&candidates);
    let builtin = crate::entity::resolve_entities(&clean);
    let merged = merge_nlp_candidates(regex_only, &builtin, 0.60);
    suppress_contained_spans(merged)
}

fn min_len_for(kind: ItemKind, text: &str) -> usize {
    // Ticket refs like #42 are intentionally short; keep them past the default floor.
    if kind == ItemKind::Id
        && text.starts_with('#')
        && text[1..].chars().all(|c| c.is_ascii_digit())
    {
        2
    } else {
        MIN_LENGTH
    }
}

fn dedupe_and_rank(candidates: &[RankedItem]) -> Vec<ExtractItem> {
    let mut best: HashMap<String, RankedItem> = HashMap::new();
    for candidate in candidates {
        let mut candidate = candidate.clone();
        candidate.item.text = canonicalize(&candidate.item.text);
        let floor = min_len_for(candidate.item.kind, &candidate.item.text);
        if candidate.item.text.chars().count() < floor || junk_token(&candidate.item.text) {
            continue;
        }
        let replace = best
            .get(&candidate.item.text)
            .is_none_or(|existing| quality_score(&candidate) > quality_score(existing));
        if replace {
            best.insert(candidate.item.text.clone(), candidate);
        }
    }
    let mut values: Vec<_> = best.into_values().collect();
    // Deterministic tie-break: quality, then later offset (recency), then text.
    values.sort_by(|a, b| {
        quality_score(b)
            .cmp(&quality_score(a))
            .then_with(|| b.offset.cmp(&a.offset))
            .then_with(|| a.item.text.cmp(&b.item.text))
    });
    values.into_iter().map(|candidate| candidate.item).collect()
}

/// Drop lower-value items whose text is fully contained in a higher-value
/// specialized span (URL/email/path). Matches urlscan/xre one-item-per-span.
fn suppress_contained_spans(items: Vec<ExtractItem>) -> Vec<ExtractItem> {
    let mut kept: Vec<ExtractItem> = Vec::with_capacity(items.len());
    for item in items {
        let redundant = kept.iter().any(|owner| span_is_redundant(owner, &item));
        if !redundant {
            kept.push(item);
        }
    }
    kept
}

fn span_is_redundant(owner: &ExtractItem, item: &ExtractItem) -> bool {
    if owner.text == item.text {
        return owner.kind.rank_value() >= item.kind.rank_value();
    }
    if !owner.text.contains(&item.text) {
        return false;
    }
    matches!(
        (owner.kind, item.kind),
        (
            ItemKind::Url | ItemKind::Email | ItemKind::Path | ItemKind::Command | ItemKind::Host,
            ItemKind::Host | ItemKind::Path | ItemKind::Email | ItemKind::Word | ItemKind::Id
        ) | (ItemKind::Phrase, ItemKind::Word)
    )
}

fn quality_score(candidate: &RankedItem) -> i64 {
    let text = &candidate.item.text;
    let variety = text.chars().collect::<HashSet<_>>().len() as i64;
    candidate.offset as i64 * 10
        + candidate.item.kind.rank_value() * 1000
        + text.chars().count() as i64 * 2
        + variety * 5
}

fn strip_terminal_artifacts(text: &str) -> String {
    // Terminal control framing needs a parser, not a greedy OSC regex.
    // Symbol exclusions belong to loose-word matching, never source mutation.
    strip_ansi_escapes::strip_str(text)
}

fn canonicalize(value: &str) -> String {
    static ANSI_RESIDUE: OnceLock<Regex> = OnceLock::new();
    let residue = ANSI_RESIDUE
        .get_or_init(|| Regex::new(r"(?i)\[\d{1,3}(?:;\d{1,3})*m").expect("ANSI residue regex"));
    residue
        .replace_all(value, "")
        .trim_matches(|character: char| {
            character.is_whitespace()
                || matches!(
                    character,
                    ',' | ':' | ';' | ')' | ']' | '}' | '>' | '|' | '.'
                )
        })
        .to_string()
}

fn junk_token(value: &str) -> bool {
    static RATIO: OnceLock<Regex> = OnceLock::new();
    static ANSI: OnceLock<Regex> = OnceLock::new();
    let ratio = RATIO
        .get_or_init(|| Regex::new(r"^\d+(?:\.\d+)?(?:%|/\d+(?:\.\d+)?)$").expect("ratio regex"));
    let ansi = ANSI.get_or_init(|| Regex::new(r"^\[?\d+(?:;\d+)*m?$").expect("SGR regex"));
    value.is_empty()
        || ratio.is_match(value)
        || ansi.is_match(value)
        || value.chars().all(|character| !character.is_alphanumeric())
}

fn word_redundant_with_specialized(word: &str, specialized: &ExtractItem) -> bool {
    if specialized.text == word || specialized.text.eq_ignore_ascii_case(word) {
        return true;
    }
    if specialized.text.starts_with(word) && specialized.kind != ItemKind::Word {
        return true;
    }
    // A specialized multi-word phrase already owns its member tokens.
    if specialized.kind == ItemKind::Phrase
        && specialized
            .text
            .split_whitespace()
            .any(|part| part.eq_ignore_ascii_case(word))
    {
        return true;
    }
    match specialized.kind {
        ItemKind::Quote => specialized.text == format!("\"{word}\""),
        ItemKind::SQuote => specialized.text == format!("'{word}'"),
        _ => false,
    }
}

fn filter_urls(text: &str) -> Vec<RankedItem> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"(?i)(https?://|git@|git://|ssh://|s?ftp://|file:///|postgres(?:ql)?://|mysql://|redis://|mongodb://)([a-zA-Z0-9?=%/_.:,;~@!#$&()*+-]*)")
            .expect("url regex")
    });
    collect_joined_groups(re, text, ItemKind::Url, Some(r#"",):"#))
}

fn filter_markdown_urls(text: &str) -> Vec<RankedItem> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"(?i)\[([^\]]+)\]\((https?://[^)\s]+)\)").expect("markdown url regex")
    });
    re.captures_iter(text)
        .filter_map(|caps| {
            let m = caps.get(0)?;
            if shell_noise_line(text, m.start()) {
                return None;
            }
            let url = canonicalize(caps.get(2)?.as_str());
            (url.chars().count() >= MIN_LENGTH).then_some(RankedItem {
                item: ExtractItem {
                    text: url,
                    kind: ItemKind::Url,
                },
                offset: m.start(),
            })
        })
        .collect()
}

fn filter_emails(text: &str) -> Vec<RankedItem> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"(?i)\b[A-Z0-9._%+\-]+@[A-Z0-9.\-]+\.[A-Z]{2,24}\b").expect("email regex")
    });
    re.find_iter(text)
        .filter(|m| !shell_noise_line(text, m.start()))
        .filter(|m| {
            let v = m.as_str();
            // git@host is a transport identity inside git URLs, not mail.
            if v.to_ascii_lowercase().starts_with("git@") {
                let after = &text[m.end()..];
                if after.starts_with(':') || after.starts_with('/') {
                    return false;
                }
            }
            true
        })
        .map(|m| RankedItem {
            item: ExtractItem {
                text: m.as_str().to_string(),
                kind: ItemKind::Email,
            },
            offset: m.start(),
        })
        .collect()
}

fn filter_ips(text: &str) -> Vec<RankedItem> {
    static RE_V4: OnceLock<Regex> = OnceLock::new();
    static RE_V6: OnceLock<Regex> = OnceLock::new();
    let v4 = RE_V4.get_or_init(|| {
        Regex::new(
            r"\b(?:(?:25[0-5]|2[0-4]\d|1?\d?\d)\.){3}(?:25[0-5]|2[0-4]\d|1?\d?\d)(?::\d{1,5})?\b",
        )
        .expect("ipv4 regex")
    });
    let v6 = RE_V6.get_or_init(|| {
        Regex::new(r"(?i)\b(?:[0-9a-f]{1,4}:){2,7}[0-9a-f]{1,4}\b").expect("ipv6 regex")
    });
    let mut out = Vec::new();
    for m in v4.find_iter(text).chain(v6.find_iter(text)) {
        if shell_noise_line(text, m.start()) {
            continue;
        }
        // Reject version-like sequences that only look numeric with dots when
        // the match is short and has no port - still allow real LAN addresses.
        let text_val = m.as_str();
        if text_val.matches('.').count() == 3 || text_val.contains(':') {
            out.push(RankedItem {
                item: ExtractItem {
                    text: text_val.to_string(),
                    kind: ItemKind::Ip,
                },
                offset: m.start(),
            });
        }
    }
    out
}

fn filter_uuids(text: &str) -> Vec<RankedItem> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"(?i)\b[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\b")
            .expect("uuid regex")
    });
    re.find_iter(text)
        .filter(|m| !shell_noise_line(text, m.start()))
        .map(|m| RankedItem {
            item: ExtractItem {
                text: m.as_str().to_string(),
                kind: ItemKind::Uuid,
            },
            offset: m.start(),
        })
        .collect()
}

fn filter_hosts(text: &str) -> Vec<RankedItem> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(
            r"(?i)\b(?:[a-z0-9](?:[a-z0-9\-]{0,61}[a-z0-9])?\.)+(?:local|internal|lan|corp|home|test|dev|localhost|[a-z]{2,24})(?::\d{1,5})?\b",
        )
        .expect("host regex")
    });
    re.find_iter(text)
        .filter(|m| !shell_noise_line(text, m.start()))
        .filter(|m| {
            let value = m.as_str().trim_end_matches('.');
            !value.contains("://")
                && value.chars().any(|c| c.is_ascii_alphabetic())
                && plausible_hostname(value)
        })
        .map(|m| RankedItem {
            item: ExtractItem {
                text: m.as_str().trim_end_matches('.').to_string(),
                kind: ItemKind::Host,
            },
            offset: m.start(),
        })
        .collect()
}

fn plausible_hostname(value: &str) -> bool {
    const BAD_TLDS: &[&str] = &[
        "gz", "tar", "tgz", "zip", "bz2", "xz", "log", "txt", "md", "rs", "go", "py", "js", "ts",
        "tsx", "jsx", "json", "toml", "yaml", "yml", "lock", "png", "jpg", "jpeg", "gif", "webp",
        "svg", "bin", "exe", "dll", "so", "dylib", "o", "a", "out", "class", "jar", "war", "rpm",
        "deb", "apk", "dmg", "iso", "img", "csv", "tsv", "sql", "sh", "bash", "zsh", "fish", "ps1",
        "bat", "cmd", "conf", "cfg", "ini", "env", "bak", "tmp", "swp", "swo", "orig", "rej",
        "git",
    ];
    let host = value.split_once(':').map(|(h, _)| h).unwrap_or(value);
    let Some(tld) = host.rsplit('.').next() else {
        return false;
    };
    let tld_l = tld.to_ascii_lowercase();
    if BAD_TLDS.contains(&tld_l.as_str()) {
        return false;
    }
    // Multi-label internal hosts and real domains only.
    host.matches('.').count() >= 1
        && host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        })
}

fn filter_paths(text: &str) -> Vec<RankedItem> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        // Do not treat `:` alone as a path boundary — that leaks URL interiors.
        Regex::new(concat!(
            r#"(?i)(?:[\t\n \"'(\[<']|^)"#,
            r#"((?:~|/)?[-~A-Za-z0-9_+,.@]+/[^ \t\n\r|:\"'$%&) >\]]*)"#,
        ))
        .expect("path regex")
    });
    let mut out = Vec::new();
    for caps in re.captures_iter(text) {
        let Some(m) = caps.get(1) else { continue };
        if shell_noise_line(text, m.start()) || path_looks_like_url_interior(text, m.start()) {
            continue;
        }
        let item = canonicalize(m.as_str());
        if item.chars().count() >= MIN_LENGTH && !junk_token(&item) && plausible_path(&item) {
            out.push(RankedItem {
                item: ExtractItem {
                    text: item,
                    kind: ItemKind::Path,
                },
                offset: m.start(),
            });
        }
    }
    out
}

fn path_looks_like_url_interior(text: &str, start: usize) -> bool {
    let before = &text[..start];
    let line_start = before.rfind('\n').map_or(0, |i| i + 1);
    let line_before = &before[line_start..];
    if line_before.contains("://") {
        return true;
    }
    before.ends_with('@')
        || (before.ends_with(':') && before[..before.len().saturating_sub(1)].contains('@'))
}

fn plausible_path(value: &str) -> bool {
    if !value.contains('/') || value.starts_with("//") {
        return false;
    }
    if !value.starts_with('/')
        && !value.starts_with('~')
        && !value.starts_with("./")
        && !value.starts_with("../")
    {
        let first = value.split('/').next().unwrap_or("");
        if first.contains('.') {
            return false;
        }
        if value.ends_with(".git") {
            return false;
        }
    }
    !value
        .chars()
        .all(|character| character.is_ascii_digit() || character == '/')
        && value
            .split('/')
            .all(|part| !part.is_empty() || value.starts_with('/'))
}

fn filter_ids(text: &str) -> Vec<RankedItem> {
    static RE: OnceLock<Regex> = OnceLock::new();
    static UUID: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        // Ticket-like only: #1234 (2–5 digits; 6/8 are colors), PROJ-99, wiseman-8ure.
        Regex::new(r"(?:#\d{2,5}\b|\b[A-Z][A-Z0-9]{1,15}[_-]\d{1,8}\b|\b[a-z][a-z0-9]{1,24}-[a-z0-9]*\d[a-z0-9]{0,11}\b)")
            .expect("id regex")
    });
    let uuid = UUID.get_or_init(|| {
        Regex::new(r"(?i)\b[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\b")
            .expect("uuid for id guard")
    });
    re.find_iter(text)
        .filter(|m| !shell_noise_line(text, m.start()))
        .filter(|m| {
            let v = m.as_str();
            if v.chars()
                .all(|c| c.is_ascii_hexdigit() || c == '#' || c == '-')
                && v.contains('-')
            {
                // UUID fragments look like ticket ids; drop when inside a UUID span.
                if uuid
                    .find_iter(text)
                    .any(|u| m.start() >= u.start() && m.end() <= u.end())
                {
                    return false;
                }
            }
            if v.chars().all(|c| c.is_ascii_hexdigit() || c == '#') && v.len() >= 7 {
                return false;
            }
            // All-digit #rrggbb / #rrggbbaa colors must stay Color, not Id.
            if let Some(hex) = v.strip_prefix('#') {
                if matches!(hex.len(), 6 | 8) && hex.chars().all(|c| c.is_ascii_hexdigit()) {
                    return false;
                }
            }
            v.chars().count() >= 3
        })
        .map(|m| RankedItem {
            item: ExtractItem {
                text: m.as_str().to_string(),
                kind: ItemKind::Id,
            },
            offset: m.start(),
        })
        .collect()
}

fn filter_envs(text: &str) -> Vec<RankedItem> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"\b\$?[A-Z][A-Z0-9]{0,30}_[A-Z0-9_]{2,48}\b").expect("env regex")
    });
    re.find_iter(text)
        .filter(|m| !shell_noise_line(text, m.start()))
        .map(|m| RankedItem {
            item: ExtractItem {
                text: m.as_str().to_string(),
                kind: ItemKind::Env,
            },
            offset: m.start(),
        })
        .collect()
}

fn filter_phrases(text: &str) -> Vec<RankedItem> {
    // Lightweight title-case bi/trigrams so phrase hits still appear when the
    // entity model is disabled in tests that call filters indirectly. The
    // built-in resolver is the primary phrase source.
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(
            r"\b([A-Z][a-zA-Z0-9'’-]{1,24}(?:\s+(?:of|and|the|for|in|on|to|de|la|van)\s+)?(?:\s+[A-Z][a-zA-Z0-9'’-]{1,24}){1,4})\b",
        )
        .expect("phrase regex")
    });
    re.find_iter(text)
        .filter(|m| !shell_noise_line(text, m.start()))
        .filter(|m| m.as_str().split_whitespace().count() >= 2)
        .map(|m| RankedItem {
            item: ExtractItem {
                text: m.as_str().to_string(),
                kind: ItemKind::Phrase,
            },
            offset: m.start(),
        })
        .collect()
}

fn filter_quotes(text: &str) -> Vec<RankedItem> {
    static RE: OnceLock<Regex> = OnceLock::new();
    collect_full_match(
        RE.get_or_init(|| Regex::new(r#"\"([^\"\n\r]+)\""#).expect("quote regex")),
        text,
        ItemKind::Quote,
    )
}

fn filter_s_quotes(text: &str) -> Vec<RankedItem> {
    static RE: OnceLock<Regex> = OnceLock::new();
    collect_full_match(
        RE.get_or_init(|| Regex::new(r"'([^'\n\r]+)'").expect("s-quote regex")),
        text,
        ItemKind::SQuote,
    )
}

fn filter_code(text: &str) -> Vec<RankedItem> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r#"`([^`\n\r]+)`|\"([A-Za-z_][A-Za-z0-9_.-]*)\"\s*:"#).expect("code regex")
    });
    re.captures_iter(text)
        .filter_map(|caps| {
            let m = caps.get(0)?;
            if shell_noise_line(text, m.start()) {
                return None;
            }
            let item = if caps.get(1).is_some() {
                m.as_str().to_string()
            } else {
                caps.get(2)?.as_str().to_string()
            };
            (item.chars().count() >= MIN_LENGTH).then_some(RankedItem {
                item: ExtractItem {
                    text: canonicalize(&item),
                    kind: ItemKind::Code,
                },
                offset: m.start(),
            })
        })
        .collect()
}

fn filter_commands(text: &str) -> Vec<RankedItem> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"(?im)(?:^|[>$]\s*|\b(?:run|exec|command):\s*)((?:cargo|git|gh|herdr|npm|bun|pnpm|yarn|uv|python|node|docker|podman|kubectl|make|cmake|curl|ssh|tmux|rg|grep|ls|cd|cat|echo|rustup|systemctl|journalctl|bd|fm)\b[^\n\r]*)")
            .expect("command regex")
    });
    re.captures_iter(text)
        .filter_map(|caps| {
            let m = caps.get(1)?;
            let item = trim_command(m.as_str());
            if item.chars().count() < MIN_LENGTH || item.split_whitespace().count() < 2 {
                return None;
            }
            // Bare line-start matches need a flag, path, or host token so prose
            // like `gh shorthand 'org/repo'` is not treated as a command.
            let line_start = text[..m.start()].rfind('\n').map_or(0, |i| i + 1);
            let prefix = text[line_start..m.start()].trim_end();
            let prompted = prefix.ends_with(['$', '>'])
                || prefix.to_ascii_lowercase().ends_with("command:")
                || prefix.to_ascii_lowercase().ends_with("run:")
                || prefix.to_ascii_lowercase().ends_with("exec:");
            let has_flag = item.split_whitespace().any(|w| w.starts_with('-'));
            // Quoted owner/repo is not a shell path target.
            let has_target = item.split_whitespace().skip(1).any(|w| {
                let bare = w.trim_matches(['\'', '"', '`']);
                bare.starts_with('/')
                    || bare.starts_with('~')
                    || bare.contains('@')
                    || bare.contains("://")
            });
            if !(prompted || has_flag || has_target) {
                return None;
            }
            Some(RankedItem {
                item: ExtractItem {
                    text: item,
                    kind: ItemKind::Command,
                },
                offset: m.start(),
            })
        })
        .collect()
}

fn trim_command(value: &str) -> String {
    value.trim().trim_end_matches(['.', ';']).to_string()
}

fn filter_hashes(text: &str) -> Vec<RankedItem> {
    static RE: OnceLock<Regex> = OnceLock::new();
    static UUID: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"(?i)\b[0-9a-f]{7,64}\b").expect("hash regex"));
    let uuid = UUID.get_or_init(|| {
        Regex::new(r"(?i)\b[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\b")
            .expect("uuid for hash guard")
    });
    re.find_iter(text)
        .filter(|m| !shell_noise_line(text, m.start()))
        .filter(|m| {
            let len = m.as_str().len();
            if uuid
                .find_iter(text)
                .any(|u| m.start() >= u.start() && m.end() <= u.end())
            {
                return false;
            }
            m.as_str()
                .chars()
                .any(|c| matches!(c, 'a'..='f' | 'A'..='F'))
                || len >= 10
        })
        .map(|m| RankedItem {
            item: ExtractItem {
                text: m.as_str().to_string(),
                kind: ItemKind::Hash,
            },
            offset: m.start(),
        })
        .collect()
}

fn filter_versions(text: &str) -> Vec<RankedItem> {
    static RE: OnceLock<Regex> = OnceLock::new();
    static IPV4: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"\bv?\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?\b")
            .expect("version regex")
    });
    let ipv4 = IPV4.get_or_init(|| {
        Regex::new(
            r"\b(?:(?:25[0-5]|2[0-4]\d|1?\d?\d)\.){3}(?:25[0-5]|2[0-4]\d|1?\d?\d)(?::\d{1,5})?\b",
        )
        .expect("ipv4 for version guard")
    });
    re.find_iter(text)
        .filter(|m| !shell_noise_line(text, m.start()))
        .filter(|m| {
            !ipv4
                .find_iter(text)
                .any(|ip| m.start() >= ip.start() && m.end() <= ip.end())
        })
        .map(|m| RankedItem {
            item: ExtractItem {
                text: m.as_str().to_string(),
                kind: ItemKind::Version,
            },
            offset: m.start(),
        })
        .collect()
}

/// tmux-fastcopy parity: `0x` hex addresses stay copyable as hashes.
fn filter_hex_addrs(text: &str) -> Vec<RankedItem> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"(?i)\b0x[0-9a-f]{2,}\b").expect("hex addr regex"));
    re.find_iter(text)
        .filter(|m| !shell_noise_line(text, m.start()))
        .filter(|m| m.as_str().chars().count() >= MIN_LENGTH)
        .map(|m| RankedItem {
            item: ExtractItem {
                text: m.as_str().to_string(),
                kind: ItemKind::Hash,
            },
            offset: m.start(),
        })
        .collect()
}

/// tmux-fastcopy parity: CSS/terminal hex colors.
fn filter_hex_colors(text: &str) -> Vec<RankedItem> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        // 6/8-digit only: 3-digit forms are under the global 5-char floor.
        Regex::new(r"(?i)#(?:[0-9a-f]{6}|[0-9a-f]{8})\b").expect("hex color regex")
    });
    re.find_iter(text)
        .filter(|m| !shell_noise_line(text, m.start()))
        .map(|m| RankedItem {
            item: ExtractItem {
                text: m.as_str().to_string(),
                kind: ItemKind::Color,
            },
            offset: m.start(),
        })
        .collect()
}

/// tmux-fastcopy parity: ISO calendar dates.
fn filter_iso_dates(text: &str) -> Vec<RankedItem> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"\b(?:19|20)\d{2}-(?:0[1-9]|1[0-2])-(?:0[1-9]|[12]\d|3[01])\b")
            .expect("iso date regex")
    });
    re.find_iter(text)
        .filter(|m| !shell_noise_line(text, m.start()))
        .map(|m| RankedItem {
            item: ExtractItem {
                text: m.as_str().to_string(),
                kind: ItemKind::Date,
            },
            offset: m.start(),
        })
        .collect()
}

/// tmux-fzf-url parity: quoted `owner/repo` GitHub shorthand becomes a URL.
fn filter_github_shorthand(text: &str) -> Vec<RankedItem> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE
        .get_or_init(|| Regex::new(r#"['"]([\w-]+/[\w.-]+)['"]"#).expect("github shorthand regex"));
    re.captures_iter(text)
        .filter_map(|caps| {
            let m = caps.get(0)?;
            if shell_noise_line(text, m.start()) {
                return None;
            }
            let repo = caps.get(1)?.as_str();
            // Avoid paths that look like domain/tld pairs without a real owner shape.
            if repo.starts_with('.') || repo.ends_with('.') || !repo.contains('/') {
                return None;
            }
            let url = format!("https://github.com/{repo}");
            (url.chars().count() >= MIN_LENGTH).then_some(RankedItem {
                item: ExtractItem {
                    text: url,
                    kind: ItemKind::Url,
                },
                offset: m.start(),
            })
        })
        .collect()
}

fn filter_errors(text: &str) -> Vec<RankedItem> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"(?im)^\s*(?:.*\b(?:error|failed|failure|assert(?:ion)?|panic|traceback)\b.*)$")
            .expect("error regex")
    });
    re.find_iter(text)
        .filter_map(|m| {
            if shell_noise_line(text, m.start()) {
                return None;
            }
            let item = m.as_str().trim();
            (item.chars().count() >= MIN_LENGTH && item.chars().count() <= 240).then_some(
                RankedItem {
                    item: ExtractItem {
                        text: item.to_string(),
                        kind: ItemKind::Error,
                    },
                    offset: m.start(),
                },
            )
        })
        .collect()
}

fn filter_words(text: &str) -> Vec<RankedItem> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"[^\]\[(){}=$\u{2500}-\u{27BF}\u{E000}-\u{F8FF}⋅↴ \t\n\r]+")
            .expect("word regex")
    });
    let lstrip: &[char] = &[
        ',', ':', ';', '(', ')', '[', ']', '{', '}', '<', '>', '\'', '"', '|',
    ];
    let rstrip: &[char] = &[
        ',', ':', ';', '(', ')', '[', ']', '{', '}', '<', '>', '\'', '"', '|', '.',
    ];
    re.find_iter(text)
        .filter_map(|m| {
            if shell_noise_line(text, m.start()) {
                return None;
            }
            let item = m
                .as_str()
                .trim_start_matches(lstrip)
                .trim_end_matches(rstrip);
            (item.chars().count() >= MIN_LENGTH && !junk_token(item)).then_some(RankedItem {
                item: ExtractItem {
                    text: item.to_string(),
                    kind: ItemKind::Word,
                },
                offset: m.start(),
            })
        })
        .collect()
}

fn collect_joined_groups(
    re: &Regex,
    text: &str,
    kind: ItemKind,
    rstrip: Option<&str>,
) -> Vec<RankedItem> {
    re.captures_iter(text)
        .filter_map(|caps| {
            let m = caps.get(0)?;
            if shell_noise_line(text, m.start()) {
                return None;
            }
            let mut item = String::new();
            for index in 1..caps.len() {
                if let Some(group) = caps.get(index) {
                    item.push_str(group.as_str());
                }
            }
            if let Some(chars) = rstrip {
                while item
                    .chars()
                    .last()
                    .is_some_and(|character| chars.contains(character))
                {
                    item.pop();
                }
            }
            (item.chars().count() >= MIN_LENGTH).then_some(RankedItem {
                item: ExtractItem {
                    text: canonicalize(&item),
                    kind,
                },
                offset: m.start(),
            })
        })
        .collect()
}

fn collect_full_match(re: &Regex, text: &str, kind: ItemKind) -> Vec<RankedItem> {
    re.find_iter(text)
        .filter_map(|m| {
            if shell_noise_line(text, m.start()) {
                return None;
            }
            (m.as_str().chars().count() >= MIN_LENGTH).then_some(RankedItem {
                item: ExtractItem {
                    text: m.as_str().to_string(),
                    kind,
                },
                offset: m.start(),
            })
        })
        .collect()
}

fn shell_noise_line(text: &str, offset: usize) -> bool {
    let start = text[..offset]
        .rfind('\n')
        .map_or(0, |position| position + 1);
    let end = text[offset..]
        .find('\n')
        .map_or(text.len(), |position| offset + position);
    let line = &text[start..end];
    line.contains("HERDR_SOCKET_PATH") || line.contains("HERDR_PLUGIN_") || line.contains("printf ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_semantic_classes() {
        let items = extract_items_from_visible_text(
            "cargo test --release\ncommit 0123456789abcdef\nversion v1.2.3\nerror: failed to compile\nkey `serde_json`",
        );
        assert!(items
            .iter()
            .any(|item| item.kind == ItemKind::Command && item.text.contains("cargo test")));
        assert!(items
            .iter()
            .any(|item| item.kind == ItemKind::Hash && item.text == "0123456789abcdef"));
        assert!(items
            .iter()
            .any(|item| item.kind == ItemKind::Version && item.text == "v1.2.3"));
        assert!(items.iter().any(|item| item.kind == ItemKind::Error));
        assert!(items
            .iter()
            .any(|item| item.kind == ItemKind::Code && item.text == "`serde_json`"));
    }

    #[test]
    fn extracts_new_entity_classes() {
        let items = extract_items_from_visible_text(concat!(
            "contact jon@arcs.health\n",
            "bind 10.0.0.12:8443\n",
            "uuid 550e8400-e29b-41d4-a716-446655440000\n",
            "host api.staging.example.internal ok\n",
            "claimed wiseman-8ure and PROJ-1234 and #42\n",
            "export DATABASE_URL=postgres://x\n",
            "see [docs](https://docs.example.com/guide)\n",
            "met Jon Roosevelt at Covenant Clinics today\n",
            "artifact /srv/releases/staging/app.tar.gz\n",
        ));
        assert!(items
            .iter()
            .any(|i| i.kind == ItemKind::Email && i.text == "jon@arcs.health"));
        assert!(items
            .iter()
            .any(|i| i.kind == ItemKind::Ip && i.text.starts_with("10.0.0.12")));
        assert!(items.iter().any(|i| i.kind == ItemKind::Uuid));
        assert!(items
            .iter()
            .any(|i| i.kind == ItemKind::Host && i.text.contains("example.internal")));
        assert!(
            items
                .iter()
                .any(|i| i.kind == ItemKind::Id
                    && (i.text == "wiseman-8ure" || i.text == "PROJ-1234"))
        );
        assert!(items
            .iter()
            .any(|i| i.kind == ItemKind::Id && i.text == "#42"));
        assert!(items.iter().any(|i| i.kind == ItemKind::Env
            && (i.text == "DATABASE_URL" || i.text.contains("DATABASE_URL"))));
        assert!(items
            .iter()
            .any(|i| i.kind == ItemKind::Url && i.text.contains("docs.example.com")));
        assert!(items.iter().any(|i| i.kind == ItemKind::Phrase
            && (i.text.contains("Jon Roosevelt") || i.text.contains("Covenant Clinics"))));
        assert!(items
            .iter()
            .any(|i| i.kind == ItemKind::Url && i.text.starts_with("postgres://")));
        // Regression: no UUID slices as ids/hashes, no file-extension hosts, no IP-as-version.
        assert!(!items
            .iter()
            .any(|i| i.text == "a716-446655440000" || i.text == "e29b-41d4"));
        assert!(!items
            .iter()
            .any(|i| i.kind == ItemKind::Host && i.text.contains("tar.gz")));
        assert!(!items
            .iter()
            .any(|i| i.kind == ItemKind::Version && i.text == "10.0.0"));
        assert!(!items
            .iter()
            .any(|i| i.kind == ItemKind::Hash && i.text == "550e8400"));
    }

    #[test]
    fn removes_ansi_junk_and_canonicalizes_duplicates() {
        let items = extract_items_from_visible_text(
            "\x1b[38;5;7m/tmp/build/output.log\x1b[0m\n/tmp/build/output.log 38;5;7m 1/2 90%",
        );
        assert_eq!(
            items
                .iter()
                .filter(|item| item.text == "/tmp/build/output.log")
                .count(),
            1
        );
        assert!(!items
            .iter()
            .any(|item| item.text.contains("38;5;7m") || item.text == "1/2" || item.text == "90%"));
    }

    #[test]
    fn ranking_prefers_useful_recent_distinct_tokens() {
        let items =
            extract_items_from_visible_text("ordinary-word\nhttps://example.com/path\nlatest-word");
        assert_eq!(items.first().map(|item| item.kind), Some(ItemKind::Url));
        assert!(
            items
                .iter()
                .position(|item| item.text == "latest-word")
                .unwrap()
                < items
                    .iter()
                    .position(|item| item.text == "ordinary-word")
                    .unwrap()
        );
    }

    #[test]
    fn merge_nlp_candidates_respects_confidence_and_kind() {
        let regex = vec![ExtractItem {
            text: "thing-value".into(),
            kind: ItemKind::Word,
        }];
        let nlp = vec![NlpCandidate {
            text: "thing-value".into(),
            kind: ItemKind::Code,
            confidence: 0.9,
        }];
        let merged = merge_nlp_candidates(regex, &nlp, 0.8);
        assert_eq!(
            merged
                .iter()
                .find(|item| item.text == "thing-value")
                .unwrap()
                .kind,
            ItemKind::Code
        );
    }

    #[test]
    fn keeps_min_length_and_wrap_reconstruction() {
        let items = extract_items_from_visible_text_with_wrap_width(
            "Link https://wrap.example/split/path/to/\ncontinued.txt",
            Some(40),
        );
        assert!(items
            .iter()
            .any(|item| item.text == "https://wrap.example/split/path/to/continued.txt"));
        assert!(!extract_items_from_visible_text("hi")
            .iter()
            .any(|item| item.text == "hi"));
    }

    #[test]
    fn meaningful_unicode_and_osc_hyperlink_labels_survive_cleanup() {
        for text in ["/tmp/dir✓check/CHECKFILE.txt", "\"QUOTESYMBOL ✓ ready\""] {
            let items = extract_items_from_visible_text(text);
            assert!(items.iter().any(|item| item.text == text), "{items:?}");
        }
        for terminator in ["\x07", "\x1b\\"] {
            let label = "https://label.example.test/OSC_ST_ENTITY";
            let text =
                format!("\x1b]8;;https://destination.test/{terminator}{label}\x1b]8;;{terminator}");
            let items = extract_items_from_visible_text(&text);
            assert!(items.iter().any(|item| item.text == label), "{items:?}");
        }
    }

    #[test]
    fn stable_keys_round_trip() {
        for kind in ItemKind::all() {
            assert_eq!(ItemKind::from_stable_key(kind.stable_key()), Some(*kind));
        }
    }

    /// Adversarial corpus covering SOTA priors from tmux-fastcopy, tmux-fzf-url,
    /// extrakto, and urlscan, plus unicode / noise / overlap traps.
    #[test]
    fn adversarial_sota_parity_corpus() {
        let text = concat!(
            "ANSI \x1b[31mred\x1b[0m noise 1/2 90% ⋅↴\n",
            "ipv4 192.168.0.42:8443 and v6 fe80::1a2b:3c4d\n",
            "sha 0xdeadbeef and commit deadbeefcafebabe\n",
            "color #df8e1d and #11223344 theme\n",
            "dated 2026-11-03 and bad 2026-13-99\n",
            "mail José <jose.garcia@example.com> and mailto:ops@arcs.health\n",
            "path ~/tmp/test.txt and /home/joe/test.txt\n",
            "url https://example.com/a?q=1, and git@github.com:user/repo.git\n",
            "www.example.com/docs and markdown [x](https://docs.example.com/x)\n",
            "gh shorthand 'RooseveltAdvisors/herdr-extractor' in quotes\n",
            "uuid 550e8400-e29b-41d4-a716-446655440000\n",
            "tickets wiseman-8ure PROJ-99 #1234\n",
            "env DATABASE_URL=postgres://db:5432/app\n",
            "names: Jon Roosevelt, Covenant Clinics, Board of Directors\n",
            "command: cargo test --release\n",
            "error: failed to compile crate\n",
            "quote \"hello world\" and 'single quote'\n",
            "code `serde_json` key\n",
            "CJK 部署成功 path /tmp/日本語/file.log\n",
            "artifact app.tar.gz should not be a host\n",
        );
        let items = extract_items_from_visible_text(text);
        let has = |kind: ItemKind, pred: &dyn Fn(&str) -> bool| {
            items.iter().any(|i| i.kind == kind && pred(&i.text))
        };

        assert!(has(ItemKind::Ip, &|t| t.starts_with("192.168.0.42")));
        assert!(has(ItemKind::Hash, &|t| t.eq_ignore_ascii_case("0xdeadbeef")));
        assert!(has(ItemKind::Color, &|t| t.eq_ignore_ascii_case("#df8e1d")));
        assert!(has(ItemKind::Color, &|t| t.eq_ignore_ascii_case("#11223344")));
        assert!(!items
            .iter()
            .any(|i| i.kind == ItemKind::Id && i.text.eq_ignore_ascii_case("#11223344")));
        assert!(!items
            .iter()
            .any(|i| { i.kind == ItemKind::Command && i.text.contains("gh shorthand") }));
        assert!(has(ItemKind::Date, &|t| t == "2026-11-03"));
        assert!(!items
            .iter()
            .any(|i| i.kind == ItemKind::Date && i.text == "2026-13-99"));
        assert!(has(ItemKind::Email, &|t| t.contains("jose.garcia@")));
        assert!(has(ItemKind::Path, &|t| t.contains("tmp/test.txt")
            || t.contains("/home/joe")));
        assert!(has(ItemKind::Url, &|t| t.contains("example.com/a")));
        assert!(has(ItemKind::Url, &|t| t
            .contains("github.com/RooseveltAdvisors/herdr-extractor")));
        assert!(has(ItemKind::Uuid, &|t| t.starts_with("550e8400")));
        assert!(has(ItemKind::Id, &|t| t == "wiseman-8ure"
            || t == "PROJ-99"
            || t == "#1234"));
        assert!(has(ItemKind::Env, &|t| t.contains("DATABASE_URL")));
        assert!(has(ItemKind::Url, &|t| t.starts_with("postgres://")));
        assert!(has(ItemKind::Phrase, &|t| t.contains("Jon Roosevelt")
            || t.contains("Covenant")));
        assert!(has(ItemKind::Command, &|t| t.contains("cargo test")));
        assert!(has(ItemKind::Error, &|t| t
            .to_ascii_lowercase()
            .contains("failed")));
        assert!(has(ItemKind::Quote, &|t| t.contains("hello world")));
        assert!(has(ItemKind::Code, &|t| t.contains("serde_json")));
        assert!(!items
            .iter()
            .any(|i| i.kind == ItemKind::Host && i.text.contains("tar.gz")));
        assert!(!items.iter().any(|i| i.text == "1/2" || i.text == "90%"));
        assert!(!items
            .iter()
            .any(|i| i.kind == ItemKind::Hash && i.text == "550e8400"));
    }
}
