//! Visible-buffer token extraction (extrakto-parity subset).
//!
//! Pure logic: bounded URL / path / quote / word extraction from visible text with
//! reverse + ordered dedupe. No socket or TTY.

use regex::Regex;
use std::collections::HashSet;
use std::sync::OnceLock;
use unicode_width::UnicodeWidthStr;

/// Kind of extracted item (for tests and future filter cycling).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemKind {
    Url,
    Path,
    Quote,
    SQuote,
    Word,
}

/// One copy-eligible token from the visible buffer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtractItem {
    pub text: String,
    pub kind: ItemKind,
}

const MIN_LENGTH: usize = 5;

/// Extract the v1 item set from already-visible pane text.
///
/// Default list = path ∪ url ∪ quote ∪ s-quote ∪ word (min length 5), reversed so
/// lower/more-recent screen content appears first, then deduped preserving order.
pub fn extract_items_from_visible_text(text: &str) -> Vec<ExtractItem> {
    extract_items_from_flat(text)
}

/// Extract items after rejoining visible rows that fill the pane's wrap width.
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

fn extract_items_from_flat(text: &str) -> Vec<ExtractItem> {
    let mut specialized: Vec<ExtractItem> = Vec::new();
    specialized.extend(filter_urls(text));
    specialized.extend(filter_paths(text));
    specialized.extend(filter_quotes(text));
    specialized.extend(filter_s_quotes(text));

    // Words are merged last and therefore rank first after reverse. Drop words
    // covered by a specialized match (strict prefix from charset splits, or the
    // bare interior of a quote) so typeahead prefers the full Url/Path/Quote.
    let words: Vec<ExtractItem> = filter_words(text)
        .into_iter()
        .filter(|word| {
            !specialized
                .iter()
                .any(|item| word_redundant_with_specialized(&word.text, item))
        })
        .collect();

    let mut raw: Vec<ExtractItem> = Vec::new();
    raw.extend(specialized);
    raw.extend(words);

    raw.reverse();

    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for item in raw {
        if seen.insert(item.text.clone()) {
            out.push(item);
        }
    }
    out
}

fn word_redundant_with_specialized(word: &str, specialized: &ExtractItem) -> bool {
    // Exact dup or truncated word charset split (e.g. URL cut at '=').
    if specialized.text.starts_with(word) {
        return true;
    }
    match specialized.kind {
        ItemKind::Quote => specialized.text == format!("\"{word}\""),
        ItemKind::SQuote => specialized.text == format!("'{word}'"),
        ItemKind::Url | ItemKind::Path | ItemKind::Word => false,
    }
}

fn filter_urls(text: &str) -> Vec<ExtractItem> {
    // Extrakto: (https?://|git@|git://|ssh://|s*ftp://|file:///)(body)
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(
            r"(?i)(https?://|git@|git://|ssh://|s?ftp://|file:///)([a-zA-Z0-9?=%/_.:,;~@!#$&()*+-]*)",
        )
        .expect("url regex")
    });
    collect_joined_groups(re, text, ItemKind::Url, Some(r#"",):"#))
}

fn filter_paths(text: &str) -> Vec<ExtractItem> {
    // Extrakto-parity path: lead-in + path body containing at least one `/`.
    // Haystack is prefixed with newline so column-0 paths still match.
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(concat!(
            r#"(?i)(?:[\t\n "'(\[<':]|^)"#,
            r#"((?:~|/)?[-~A-Za-z0-9_+,.]+/[^ \t\n\r|:"'$%&)>\]]*)"#,
        ))
        .expect("path regex")
    });
    static EXCLUDE: OnceLock<Regex> = OnceLock::new();
    let exclude =
        EXCLUDE.get_or_init(|| Regex::new(r"(?i)[kmgbps]/s$|^\d+/\d+$").expect("path exclude"));

    let mut out = Vec::new();
    let haystack = format!("\n{text}");
    for caps in re.captures_iter(&haystack) {
        let Some(m) = caps.get(1) else {
            continue;
        };
        let item = m
            .as_str()
            .trim_end_matches(['"', ',', ')', ':'])
            .to_string();
        if item.chars().count() < MIN_LENGTH {
            continue;
        }
        if exclude.is_match(&item) {
            continue;
        }
        out.push(ExtractItem {
            text: item,
            kind: ItemKind::Path,
        });
    }
    out
}

fn filter_quotes(text: &str) -> Vec<ExtractItem> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r#""([^"\n\r]+)""#).expect("quote regex"));
    collect_full_match(re, text, ItemKind::Quote)
}

fn filter_s_quotes(text: &str) -> Vec<ExtractItem> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"'([^'\n\r]+)'").expect("s-quote regex"));
    collect_full_match(re, text, ItemKind::SQuote)
}

fn filter_words(text: &str) -> Vec<ExtractItem> {
    // Extrakto word charset: anything but [](){}=$ box-drawing private-use whitespace.
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"[^\]\[(){}=$\u{2500}-\u{27BF}\u{E000}-\u{F8FF}⋅↴│ \t\n\r]+")
            .expect("word regex")
    });
    let lstrip: &[char] = &[
        ',', ':', ';', '(', ')', '[', ']', '{', '}', '<', '>', '\'', '"', '|',
    ];
    let rstrip: &[char] = &[
        ',', ':', ';', '(', ')', '[', ']', '{', '}', '<', '>', '\'', '"', '|', '.',
    ];
    let mut out = Vec::new();
    for m in re.find_iter(text) {
        let item = m
            .as_str()
            .trim_start_matches(lstrip)
            .trim_end_matches(rstrip);
        if item.chars().count() < MIN_LENGTH {
            continue;
        }
        out.push(ExtractItem {
            text: item.to_string(),
            kind: ItemKind::Word,
        });
    }
    out
}

fn collect_joined_groups(
    re: &Regex,
    text: &str,
    kind: ItemKind,
    rstrip: Option<&str>,
) -> Vec<ExtractItem> {
    let mut out = Vec::new();
    for caps in re.captures_iter(text) {
        let mut item = String::new();
        for i in 1..caps.len() {
            if let Some(g) = caps.get(i) {
                item.push_str(g.as_str());
            }
        }
        if let Some(chars) = rstrip {
            while item.chars().last().is_some_and(|c| chars.contains(c)) {
                item.pop();
            }
        }
        if item.chars().count() < MIN_LENGTH {
            continue;
        }
        out.push(ExtractItem { text: item, kind });
    }
    out
}

fn collect_full_match(re: &Regex, text: &str, kind: ItemKind) -> Vec<ExtractItem> {
    let mut out = Vec::new();
    for m in re.find_iter(text) {
        let item = m.as_str();
        if item.chars().count() < MIN_LENGTH {
            continue;
        }
        out.push(ExtractItem {
            text: item.to_string(),
            kind,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn texts(items: &[ExtractItem]) -> Vec<&str> {
        items.iter().map(|i| i.text.as_str()).collect()
    }

    fn fixture_visible() -> &'static str {
        "\
# on-screen decoys
DECOY_LINE_076 https://decoy-76.invalid/x /decoy/path/76.txt
DECOY_LINE_077 https://decoy-77.invalid/x /decoy/path/77.txt
Visit https://example.com/docs/api?v=1 for docs.
Config at ~/projects/herdr-leap/config.toml and /var/log/herdr/server.log
Run: cargo test --release --locked
double: \"hello world value\"
single: 'single-quoted-token'
clone git@github.com:RooseveltAdvisors/herdr-leap.git
see path/with/relative/file.rs
short hi ordinary-long-word-here
curl https://cdn.example.org/v2/asset.tar.gz
# END_FIXTURE"
    }

    #[test]
    fn extracts_urls_paths_quotes_words_from_fixture() {
        let items = extract_items_from_visible_text(fixture_visible());
        let t = texts(&items);
        for expected in [
            "https://example.com/docs/api?v=1",
            "https://cdn.example.org/v2/asset.tar.gz",
            "git@github.com:RooseveltAdvisors/herdr-leap.git",
            "~/projects/herdr-leap/config.toml",
            "path/with/relative/file.rs",
            "\"hello world value\"",
            "'single-quoted-token'",
            "ordinary-long-word-here",
        ] {
            assert!(t.contains(&expected), "missing {expected:?} in {t:?}");
        }
        assert!(
            !t.iter().any(|s| s.contains("decoy-0")),
            "off-screen decoy-0 must not appear: {t:?}"
        );
    }

    #[test]
    fn exact_width_rows_are_not_joined_without_wrap_metadata() {
        let items = extract_items_from_visible_text("abcde\nfghij");
        let t = texts(&items);
        assert!(t.contains(&"abcde"), "got {t:?}");
        assert!(t.contains(&"fghij"), "got {t:?}");
        assert!(!t.contains(&"abcdefghij"), "got {t:?}");
    }

    #[test]
    fn paths_capture_complete_extrakto_tail_tokens() {
        let paths = filter_paths("/tmp/foo=bar/baz /tmp/über/file /tmp/@scope/package");
        let paths: Vec<_> = paths.iter().map(|item| item.text.as_str()).collect();
        for expected in ["/tmp/foo=bar/baz", "/tmp/über/file", "/tmp/@scope/package"] {
            assert!(
                paths.contains(&expected),
                "missing {expected:?} in {paths:?}"
            );
        }
        assert!(!paths.contains(&"/tmp/foo"), "truncated path in {paths:?}");
        assert!(!paths.contains(&"/tmp/"), "truncated path in {paths:?}");
    }

    #[test]
    fn min_length_5_drops_short_words() {
        let items = extract_items_from_visible_text("short hi ordinary-long-word-here");
        let t = texts(&items);
        assert!(!t.contains(&"hi"));
        assert!(t.contains(&"ordinary-long-word-here"));
        // "short" is exactly 5 and should survive as a word.
        assert!(t.contains(&"short"));
    }

    #[test]
    fn dedupes_preserving_order_after_reverse() {
        let text =
            "see /tmp/alpha/file.txt once\nand /tmp/alpha/file.txt twice\nzz-bottom-unique-token";
        let items = extract_items_from_visible_text(text);
        let paths: Vec<_> = items
            .iter()
            .filter(|i| i.text.contains("/tmp/alpha/file.txt"))
            .collect();
        assert_eq!(paths.len(), 1, "expected one deduped path, got {paths:?}");
        // Bottom content should rank earlier after reverse+dedupe.
        let t = texts(&items);
        let bottom = t
            .iter()
            .position(|s| *s == "zz-bottom-unique-token")
            .expect("bottom token");
        let path_pos = t
            .iter()
            .position(|s| s.contains("/tmp/alpha/file.txt"))
            .expect("path");
        assert!(bottom < path_pos, "bottom-first order broken: {t:?}");
    }

    #[test]
    fn word_uses_distinct_leading_and_trailing_strip_sets() {
        let items = extract_items_from_visible_text("edit .gitignore with plugin.");
        let t = texts(&items);
        assert!(t.contains(&".gitignore"), "got {t:?}");
        assert!(!t.contains(&"gitignore"), "got {t:?}");
        assert!(t.contains(&"plugin"), "got {t:?}");
        assert!(!t.iter().any(|s| s.ends_with('.')), "got {t:?}");
    }

    #[test]
    fn empty_visible_text_extracts_nothing() {
        assert!(extract_items_from_visible_text("").is_empty());
    }

    #[test]
    fn specialized_tokens_outrank_truncated_word_prefixes() {
        let text = "\
Visit https://example.com/docs/api?v=1 for docs.\n\
single: 'single-quoted-token'\n\
double: \"hello-world-value\"\n";
        let items = extract_items_from_visible_text(text);
        let t = texts(&items);

        assert!(
            t.contains(&"https://example.com/docs/api?v=1"),
            "full URL missing: {t:?}"
        );
        assert!(
            !t.contains(&"https://example.com/docs/api?v"),
            "truncated URL word should be suppressed: {t:?}"
        );
        assert!(
            t.contains(&"'single-quoted-token'"),
            "single quote missing: {t:?}"
        );
        assert!(
            !t.contains(&"single-quoted-token"),
            "bare single-quote interior should be suppressed: {t:?}"
        );
        assert!(
            t.contains(&"\"hello-world-value\""),
            "double quote missing: {t:?}"
        );
        assert!(
            !t.contains(&"hello-world-value"),
            "bare double-quote interior should be suppressed: {t:?}"
        );

        let url = items
            .iter()
            .find(|i| i.text == "https://example.com/docs/api?v=1")
            .expect("url item");
        assert_eq!(url.kind, ItemKind::Url);
        let sq = items
            .iter()
            .find(|i| i.text == "'single-quoted-token'")
            .expect("squote item");
        assert_eq!(sq.kind, ItemKind::SQuote);
    }

    #[test]
    fn path_exclude_drops_transfer_speeds_and_page_fractions_only() {
        let paths = filter_paths(
            "see path/file.rs /tmp/logs /tmp/app /tmp/foo=bar/apps and 5k/s 12m/s 3b/s page 1/2",
        );
        let paths: Vec<_> = paths.iter().map(|item| item.text.as_str()).collect();
        for expected in ["path/file.rs", "/tmp/logs", "/tmp/app", "/tmp/foo=bar/apps"] {
            assert!(
                paths.contains(&expected),
                "missing legitimate path {expected:?} in {paths:?}"
            );
        }
        for excluded in ["5k/s", "12m/s", "3b/s", "1/2"] {
            assert!(
                !paths.contains(&excluded),
                "excluded token {excluded:?} leaked into {paths:?}"
            );
        }
    }
}
