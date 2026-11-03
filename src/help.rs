//! Searchable `?` help overlay (herdr-recall parity).

use crate::extract_app::FieldMode;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HelpEntry {
    pub key: &'static str,
    pub desc: &'static str,
}

/// Full key reference for the extract picker.
pub fn all_help_entries() -> &'static [HelpEntry] {
    &[
        HelpEntry {
            key: "i / a / I / A",
            desc: "enter insert mode to type filter (at cursor / after / line start / line end)",
        },
        HelpEntry {
            key: "esc / jj",
            desc: "leave insert → normal; close help; cancel from normal with empty query",
        },
        HelpEntry {
            key: "enter",
            desc: "copy selection (or all space-marked items)",
        },
        HelpEntry {
            key: "j k / arrows",
            desc: "move selection in the list (normal mode)",
        },
        HelpEntry {
            key: "gg / G",
            desc: "jump to first / last list row (normal mode)",
        },
        HelpEntry {
            key: "ctrl-d / ctrl-u",
            desc: "page down / page up in the list (normal mode)",
        },
        HelpEntry {
            key: "space",
            desc: "mark or unmark row for multi-copy (normal mode)",
        },
        HelpEntry {
            key: "tab / ctrl-g",
            desc: "cycle scope: SCROLLBACK → SCREEN (viewport) → GLOBAL → …",
        },
        HelpEntry {
            key: "s v / S / t  (normal)",
            desc: "jump scope: s|v=SCREEN · S=SCROLLBACK · t=GLOBAL (normal mode)",
        },
        HelpEntry {
            key: "ctrl-k / ctrl-j",
            desc: "cycle kind filter forward / back",
        },
        HelpEntry {
            key: "filter typing",
            desc: "in insert: type keywords; space = AND; url / @mail / path:foo = quick kind",
        },
        HelpEntry {
            key: "w b e",
            desc: "word forward / back / end inside the filter field (normal mode)",
        },
        HelpEntry {
            key: "0 $",
            desc: "start / end of filter field (normal mode)",
        },
        HelpEntry {
            key: "x / X",
            desc: "delete under / before cursor in filter field (normal mode)",
        },
        HelpEntry {
            key: "dd / D",
            desc: "clear filter query (normal mode)",
        },
        HelpEntry {
            key: "ctrl-w / ctrl-u",
            desc: "delete word / clear query (insert mode)",
        },
        HelpEntry {
            key: "?",
            desc: "open or close this help overlay",
        },
        HelpEntry {
            key: "/  (in help)",
            desc: "search help keys and descriptions",
        },
        HelpEntry {
            key: "ctrl-c",
            desc: "cancel and close the picker from anywhere",
        },
    ]
}

pub fn filter_help_entries(query: &str) -> Vec<&'static HelpEntry> {
    let q = query.trim().to_ascii_lowercase();
    all_help_entries()
        .iter()
        .filter(|entry| {
            if q.is_empty() {
                return true;
            }
            entry.key.to_ascii_lowercase().contains(&q)
                || entry.desc.to_ascii_lowercase().contains(&q)
        })
        .collect()
}

pub fn mode_badge(mode: FieldMode) -> (&'static str, &'static str) {
    match mode {
        FieldMode::Insert => ("INSERT", "type to filter"),
        FieldMode::Normal => ("NORMAL", "i type · j/k move · ? help"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_lists_core_bindings_and_filters() {
        let all = all_help_entries();
        assert!(all.len() >= 12);
        assert!(all.iter().any(|e| e.key.contains("?")));
        assert!(all.iter().any(|e| e.key.contains("i / a")));
        let filtered = filter_help_entries("mark");
        assert!(filtered.iter().any(|e| e.desc.contains("mark")));
    }
}
