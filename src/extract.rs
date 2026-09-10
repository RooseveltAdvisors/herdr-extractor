//! Pure scrollback token extraction.
//!
//! The extractor deliberately has no terminal or socket dependencies. It
//! removes terminal debris before matching and ranks stable semantic tokens.

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
            _ => None,
        }
    }

    pub fn rank_value(self) -> i64 {
        match self {
            Self::Url | Self::Path | Self::Error => 7,
            Self::Command | Self::Hash | Self::Version => 5,
            Self::Quote | Self::SQuote | Self::Code => 3,
            Self::Word => 1,
        }
    }
}

/// One copy-eligible token from pane scrollback.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtractItem {
    pub text: String,
    pub kind: ItemKind,
}

/// A candidate returned by an optional NLP sidecar.
#[derive(Clone, Debug, PartialEq)]
pub struct NlpCandidate {
    pub text: String,
    pub kind: ItemKind,
    pub confidence: f32,
}

const MIN_LENGTH: usize = 5;

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

/// Merge structured sidecar candidates with regex candidates. Candidates below
/// the threshold are ignored; an accepted candidate replaces an item with the
/// same canonical text so NLP can improve its semantic class.
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
        if text.chars().count() < MIN_LENGTH || junk_token(&text) {
            continue;
        }
        if let Some(existing) = merged.iter_mut().find(|item| item.text == text) {
            existing.kind = candidate.kind;
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
    candidates.extend(filter_paths(&clean));
    candidates.extend(filter_quotes(&clean));
    candidates.extend(filter_s_quotes(&clean));
    candidates.extend(filter_code(&clean));
    candidates.extend(filter_commands(&clean));
    candidates.extend(filter_hashes(&clean));
    candidates.extend(filter_versions(&clean));
    candidates.extend(filter_errors(&clean));

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
    dedupe_and_rank(&candidates)
}

fn dedupe_and_rank(candidates: &[RankedItem]) -> Vec<ExtractItem> {
    let mut best: HashMap<String, RankedItem> = HashMap::new();
    for candidate in candidates {
        let mut candidate = candidate.clone();
        candidate.item.text = canonicalize(&candidate.item.text);
        if candidate.item.text.chars().count() < MIN_LENGTH || junk_token(&candidate.item.text) {
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
    values.sort_by_key(|candidate| std::cmp::Reverse(quality_score(candidate)));
    values.into_iter().map(|candidate| candidate.item).collect()
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
    static ANSI: OnceLock<Regex> = OnceLock::new();
    let ansi = ANSI.get_or_init(|| {
        Regex::new(r"\x1b(?:\[[0-?]*[ -/]*[@-~]|\][^\x07]*(?:\x07|\x1b\\))").expect("ANSI regex")
    });
    ansi.replace_all(text, "")
        .chars()
        .filter(|character| {
            !matches!(
                *character,
                '\u{2500}'..='\u{27bf}' | '\u{e000}'..='\u{f8ff}' | '⋅' | '↴'
            )
        })
        .collect()
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
        || value.chars().any(|character| {
            matches!(
                character,
                '\u{2500}'..='\u{27bf}' | '\u{e000}'..='\u{f8ff}'
            )
        })
}

fn word_redundant_with_specialized(word: &str, specialized: &ExtractItem) -> bool {
    if specialized.text.starts_with(word) {
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
        Regex::new(r"(?i)(https?://|git@|git://|ssh://|s?ftp://|file:///)([a-zA-Z0-9?=%/_.:,;~@!#$&()*+-]*)")
            .expect("url regex")
    });
    collect_joined_groups(re, text, ItemKind::Url, Some(r#"",):"#))
}

fn filter_paths(text: &str) -> Vec<RankedItem> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(concat!(
            r#"(?i)(?:[\t\n \"'(\[<':]|^)"#,
            r#"((?:~|/)?[-~A-Za-z0-9_+,.@]+/[^ \t\n\r|:\"'$%&) >\]]*)"#,
        ))
        .expect("path regex")
    });
    let mut out = Vec::new();
    for caps in re.captures_iter(text) {
        let Some(m) = caps.get(1) else { continue };
        if shell_noise_line(text, m.start()) {
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

fn plausible_path(value: &str) -> bool {
    value.contains('/')
        && !value.starts_with("//")
        && !value
            .chars()
            .all(|character| character.is_ascii_digit() || character == '/')
        && value
            .split('/')
            .all(|part| !part.is_empty() || value.starts_with('/'))
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
        Regex::new(r"(?im)(?:^|[>$]\s*|\b(?:run|exec|command):\s*)((?:cargo|git|gh|herdr|npm|bun|pnpm|yarn|uv|python|node|docker|podman|kubectl|make|cmake|curl|ssh|tmux|rg|grep|ls|cd|cat|echo|rustup)\b[^\n\r]*)")
            .expect("command regex")
    });
    re.captures_iter(text)
        .filter_map(|caps| {
            let m = caps.get(1)?;
            let item = trim_command(m.as_str());
            (item.chars().count() >= MIN_LENGTH && item.split_whitespace().count() >= 2).then_some(
                RankedItem {
                    item: ExtractItem {
                        text: item,
                        kind: ItemKind::Command,
                    },
                    offset: m.start(),
                },
            )
        })
        .collect()
}

fn trim_command(value: &str) -> String {
    value.trim().trim_end_matches(['.', ';']).to_string()
}

fn filter_hashes(text: &str) -> Vec<RankedItem> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"(?i)\b[0-9a-f]{7,64}\b").expect("hash regex"));
    re.find_iter(text)
        .filter(|m| !shell_noise_line(text, m.start()))
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
    let re = RE.get_or_init(|| {
        Regex::new(r"\bv?\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?\b")
            .expect("version regex")
    });
    re.find_iter(text)
        .filter(|m| !shell_noise_line(text, m.start()))
        .map(|m| RankedItem {
            item: ExtractItem {
                text: m.as_str().to_string(),
                kind: ItemKind::Version,
            },
            offset: m.start(),
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
        let items = extract_items_from_visible_text("cargo test --release\ncommit 0123456789abcdef\nversion v1.2.3\nerror: failed to compile\nkey `serde_json`");
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
    fn stable_keys_round_trip() {
        for kind in [
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
        ] {
            assert_eq!(ItemKind::from_stable_key(kind.stable_key()), Some(kind));
        }
    }
}
