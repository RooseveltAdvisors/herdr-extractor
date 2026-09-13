//! Interactive typeahead list for the scrollback extractor.
//!
//! Pure state machine: filter items by query, move the selection, copy or cancel.
//! No terminal I/O — fully unit-testable.

use crate::extract::{ExtractItem, ItemKind};
use crate::theme::Theme;
use crate::Outcome;

const ESC: char = '\u{1b}';
const CTRL_C: char = '\u{3}';
const BACKSPACE_BS: char = '\u{8}';
const BACKSPACE_DEL: char = '\u{7f}';
const ENTER: char = '\n';
const TAB: char = '\t';
const UP: char = '\u{11}'; // DC1 — internal sentinel for Up
const DOWN: char = '\u{12}'; // DC2 — internal sentinel for Down

/// Which data source the item list was extracted from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExtractMode {
    /// The focused pane's retained scrollback.
    Scrollback,
    /// The pane's full saved session history (transcript-grade coverage).
    Global,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExtractionEngine {
    Regex,
    Nlp,
}

impl ExtractionEngine {
    pub fn name(self) -> &'static str {
        match self {
            Self::Regex => "regex",
            Self::Nlp => "nlp",
        }
    }
}

impl ExtractMode {
    pub fn toggle(self) -> Self {
        match self {
            Self::Scrollback => Self::Global,
            Self::Global => Self::Scrollback,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Scrollback => "scrollback",
            Self::Global => "global",
        }
    }
}

/// Restrict the list to one semantic family. Cycles with Shift-Tab / Ctrl-t.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum KindFilter {
    #[default]
    All,
    Path,
    Url,
    Word,
    /// Everything that is not path/url/word (command, hash, quote, …).
    Other,
}

impl KindFilter {
    pub fn cycle(self) -> Self {
        match self {
            Self::All => Self::Path,
            Self::Path => Self::Url,
            Self::Url => Self::Word,
            Self::Word => Self::Other,
            Self::Other => Self::All,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Path => "path",
            Self::Url => "url",
            Self::Word => "word",
            Self::Other => "other",
        }
    }

    pub fn matches(self, kind: ItemKind) -> bool {
        match self {
            Self::All => true,
            Self::Path => kind == ItemKind::Path,
            Self::Url => kind == ItemKind::Url,
            Self::Word => kind == ItemKind::Word,
            Self::Other => !matches!(kind, ItemKind::Path | ItemKind::Url | ItemKind::Word),
        }
    }
}

/// Inputs the extract TUI maps onto the pure state machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExtractInput {
    Char(char),
    Backspace,
    Enter,
    Up,
    Down,
    Esc,
    CtrlC,
    SwitchMode,
    CycleKindFilter,
}

impl ExtractInput {
    /// Map a control or printable character into an input for char-only drivers.
    pub fn from_char(ch: char) -> Self {
        match ch {
            ESC => Self::Esc,
            CTRL_C => Self::CtrlC,
            BACKSPACE_BS | BACKSPACE_DEL => Self::Backspace,
            ENTER | '\r' => Self::Enter,
            TAB => Self::SwitchMode,
            UP => Self::Up,
            DOWN => Self::Down,
            '\u{14}' => Self::CycleKindFilter, // DC4 — cycle_kind_sentinel
            other => Self::Char(other),
        }
    }

    /// Sentinel for char-only drivers that cannot emit BackTab / Ctrl-t.
    pub fn cycle_kind_sentinel() -> char {
        '\u{14}' // DC4
    }

    pub fn up_sentinel() -> char {
        UP
    }

    pub fn down_sentinel() -> char {
        DOWN
    }

    pub fn enter_sentinel() -> char {
        ENTER
    }
}

/// Typeahead-filtered item list.
pub struct ExtractApp {
    items: Vec<ExtractItem>,
    /// Fuzzy matches for the current query in screen order.
    filtered: Vec<FuzzyMatch>,
    query: String,
    /// Index into `filtered`.
    selected: usize,
    message: Option<String>,
    mode: ExtractMode,
    engine: ExtractionEngine,
    kind_filter: KindFilter,
    theme: Theme,
}

impl ExtractApp {
    pub fn new(items: Vec<ExtractItem>, theme: Theme) -> Self {
        let mut app = Self {
            items,
            filtered: Vec::new(),
            query: String::new(),
            selected: 0,
            message: None,
            mode: ExtractMode::Scrollback,
            engine: ExtractionEngine::Regex,
            kind_filter: KindFilter::All,
            theme,
        };
        app.refilter();
        app
    }

    /// Builder: start in a specific data mode (for example the transcript
    /// entrypoint opens straight into global mode).
    pub fn in_mode(mut self, mode: ExtractMode) -> Self {
        self.mode = mode;
        self
    }

    pub fn from_visible_text(text: &str, theme: Theme) -> Self {
        Self::new(crate::extract::extract_items_from_visible_text(text), theme)
    }

    pub fn from_visible_text_with_wrap_width(
        text: &str,
        wrap_width: Option<usize>,
        theme: Theme,
    ) -> Self {
        Self::new(
            crate::extract::extract_items_from_visible_text_with_wrap_width(text, wrap_width),
            theme,
        )
    }

    pub fn handle_input(&mut self, input: ExtractInput) -> Outcome {
        match input {
            ExtractInput::Esc | ExtractInput::CtrlC => Outcome::Cancel,
            ExtractInput::Enter => self.confirm(),
            ExtractInput::Backspace => {
                self.query.pop();
                self.refilter();
                Outcome::Continue
            }
            ExtractInput::Up => {
                self.move_sel(-1);
                Outcome::Continue
            }
            ExtractInput::Down => {
                self.move_sel(1);
                Outcome::Continue
            }
            ExtractInput::SwitchMode => Outcome::SwitchMode(self.mode.toggle()),
            ExtractInput::CycleKindFilter => {
                self.kind_filter = self.kind_filter.cycle();
                self.refilter();
                Outcome::Continue
            }
            ExtractInput::Char(ch) => {
                if ch.is_control() {
                    return Outcome::Continue;
                }
                self.query.push(ch);
                self.refilter();
                Outcome::Continue
            }
        }
    }

    /// Convenience for tests and simple char-only drivers.
    pub fn handle_char(&mut self, ch: char) -> Outcome {
        self.handle_input(ExtractInput::from_char(ch))
    }

    fn confirm(&self) -> Outcome {
        match self.selected_item() {
            Some(item) => Outcome::Copy(item.text.clone()),
            None => Outcome::Continue,
        }
    }

    fn move_sel(&mut self, delta: isize) {
        if self.filtered.is_empty() {
            self.selected = 0;
            return;
        }
        let len = self.filtered.len() as isize;
        let cur = self.selected as isize;
        let next = (cur + delta).rem_euclid(len);
        self.selected = next as usize;
    }

    fn refilter(&mut self) {
        let selected_item = self.filtered.get(self.selected).map(|item| item.index);
        self.filtered = fuzzy_matches(&self.items, &self.query)
            .into_iter()
            .filter(|item_match| {
                self.items
                    .get(item_match.index)
                    .is_some_and(|item| self.kind_filter.matches(item.kind))
            })
            .collect();
        if self.filtered.is_empty() {
            self.selected = 0;
            self.message = Some("no matches".to_string());
        } else {
            self.message = None;
            self.selected = selected_item
                .and_then(|item| {
                    self.filtered
                        .iter()
                        .position(|candidate| candidate.index == item)
                })
                .unwrap_or(0);
        }
    }

    pub fn selected_item(&self) -> Option<&ExtractItem> {
        self.filtered
            .get(self.selected)
            .and_then(|item| self.items.get(item.index))
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }

    pub fn theme(&self) -> &Theme {
        &self.theme
    }

    pub fn mode(&self) -> ExtractMode {
        self.mode
    }

    pub fn engine(&self) -> ExtractionEngine {
        self.engine
    }

    pub fn kind_filter(&self) -> KindFilter {
        self.kind_filter
    }

    pub fn set_engine(&mut self, engine: ExtractionEngine) {
        self.engine = engine;
    }

    /// Apply a mode switch: swap the item list, keep the current filter query
    /// and restore the selection by item text when it still matches.
    pub fn apply_mode(&mut self, mode: ExtractMode, items: Vec<ExtractItem>) {
        let selected_text = self.selected_item().map(|item| item.text.clone());
        self.mode = mode;
        self.items = items;
        self.refilter();
        if let Some(text) = selected_text {
            if let Some(position) = self
                .filtered
                .iter()
                .position(|item| self.items[item.index].text == text)
            {
                self.selected = position;
            }
        }
    }

    /// Show a transient status message (for example while re-reading a mode).
    pub fn set_message(&mut self, message: Option<String>) {
        self.message = message;
    }

    pub fn selected_index(&self) -> usize {
        self.selected
    }

    pub fn filtered_count(&self) -> usize {
        self.filtered.len()
    }

    pub fn total_count(&self) -> usize {
        self.items.len()
    }

    /// Items in the active kind filter before the typeahead query.
    pub fn pool_count(&self) -> usize {
        self.items
            .iter()
            .filter(|item| self.kind_filter.matches(item.kind))
            .count()
    }

    /// Filtered items in display order: `(is_selected, text)`.
    pub fn visible_rows(&self) -> Vec<(bool, &str)> {
        self.filtered
            .iter()
            .enumerate()
            .filter_map(|(pos, item_match)| {
                self.items
                    .get(item_match.index)
                    .map(|item| (pos == self.selected, item.text.as_str()))
            })
            .collect()
    }

    /// Filtered items in display order with the character positions to highlight.
    pub fn visible_matches(&self) -> Vec<(bool, &str, &[usize])> {
        self.filtered
            .iter()
            .enumerate()
            .filter_map(|(pos, item_match)| {
                self.items.get(item_match.index).map(|item| {
                    (
                        pos == self.selected,
                        item.text.as_str(),
                        item_match.positions.as_slice(),
                    )
                })
            })
            .collect()
    }

    pub fn visible_match_items(&self) -> Vec<(bool, &ExtractItem, &[usize])> {
        self.filtered
            .iter()
            .enumerate()
            .filter_map(|(pos, item_match)| {
                self.items
                    .get(item_match.index)
                    .map(|item| (pos == self.selected, item, item_match.positions.as_slice()))
            })
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FuzzyMatch {
    index: usize,
    score: i32,
    positions: Vec<usize>,
}

/// Match a query as a case-aware subsequence, favoring word starts and runs.
/// Lowercase queries are case-insensitive; an uppercase query is smart-case.
fn fuzzy_matches(items: &[ExtractItem], query: &str) -> Vec<FuzzyMatch> {
    let mut matches: Vec<_> = items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            fuzzy_match(&item.text, query).map(|(score, positions)| FuzzyMatch {
                index,
                score,
                positions,
            })
        })
        .collect();
    matches.sort_by_key(|item| std::cmp::Reverse(item.score));
    matches
}

fn fuzzy_match(text: &str, query: &str) -> Option<(i32, Vec<usize>)> {
    let query_chars: Vec<char> = query.chars().collect();
    let text_chars: Vec<char> = text.chars().collect();
    if query_chars.is_empty() {
        return Some((0, Vec::new()));
    }
    if text_chars.is_empty() || query_chars.len() > text_chars.len() {
        return None;
    }

    let smart_case = query_chars.iter().any(|character| character.is_uppercase());
    let mut scores = vec![vec![None; text_chars.len()]; query_chars.len()];
    let mut previous = vec![vec![None; text_chars.len()]; query_chars.len()];

    for (query_index, query_char) in query_chars.iter().enumerate() {
        for (text_index, text_char) in text_chars.iter().enumerate() {
            if !chars_match(*query_char, *text_char, smart_case) {
                continue;
            }
            let base = 10
                + if is_word_start(&text_chars, text_index) {
                    15
                } else {
                    0
                };
            if query_index == 0 {
                scores[query_index][text_index] = Some(base);
                continue;
            }
            let mut best: Option<(i32, usize)> = None;
            for (previous_index, previous_score) in
                scores[query_index - 1].iter().enumerate().take(text_index)
            {
                let Some(previous_score) = *previous_score else {
                    continue;
                };
                let run_bonus = if text_index == previous_index + 1 {
                    20
                } else {
                    0
                };
                let candidate = previous_score + base + run_bonus;
                if best.is_none_or(|(score, _)| candidate > score) {
                    best = Some((candidate, previous_index));
                }
            }
            if let Some((score, previous_index)) = best {
                scores[query_index][text_index] = Some(score);
                previous[query_index][text_index] = Some(previous_index);
            }
        }
    }

    let (score, mut text_index) = scores[query_chars.len() - 1]
        .iter()
        .enumerate()
        .filter_map(|(index, score)| score.map(|score| (score, index)))
        .max_by_key(|(score, index)| (*score, std::cmp::Reverse(*index)))?;
    let mut positions = vec![text_index];
    for query_index in (1..query_chars.len()).rev() {
        text_index = previous[query_index][text_index]?;
        positions.push(text_index);
    }
    positions.reverse();
    Some((score, positions))
}

fn chars_match(query: char, text: char, smart_case: bool) -> bool {
    if smart_case {
        query == text
    } else {
        query.eq_ignore_ascii_case(&text)
    }
}

fn is_word_start(text: &[char], index: usize) -> bool {
    index == 0
        || (!text[index - 1].is_alphanumeric() && text[index].is_alphanumeric())
        || (text[index].is_uppercase() && text[index - 1].is_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract::{ExtractItem, ItemKind};
    use pretty_assertions::assert_eq;

    fn items(texts: &[&str]) -> Vec<ExtractItem> {
        texts
            .iter()
            .map(|t| ExtractItem {
                text: (*t).to_string(),
                kind: ItemKind::Word,
            })
            .collect()
    }

    fn app(texts: &[&str]) -> ExtractApp {
        ExtractApp::new(items(texts), Theme::default())
    }

    #[test]
    fn empty_query_shows_all_seed_items() {
        let a = app(&["alpha-token", "beta-token", "gamma-token"]);
        assert_eq!(a.filtered_count(), 3);
        assert_eq!(a.total_count(), 3);
        let rows = a.visible_rows();
        assert_eq!(rows.len(), 3);
        assert!(rows[0].0, "first row selected by default");
        assert_eq!(rows[0].1, "alpha-token");
    }

    #[test]
    fn typeahead_filters_items() {
        let mut a = app(&[
            "https://example.com/a",
            "/tmp/path/here",
            "ordinary-long-word",
        ]);
        assert_eq!(a.handle_char('p'), Outcome::Continue);
        assert_eq!(a.handle_char('a'), Outcome::Continue);
        assert_eq!(a.handle_char('t'), Outcome::Continue);
        assert_eq!(a.query(), "pat");
        let rows = a.visible_rows();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].1, "/tmp/path/here");
    }

    #[test]
    fn enter_on_selection_copies_exact_item_string() {
        let mut a = app(&["first-item-here", "second-item-here"]);
        a.handle_input(ExtractInput::Down);
        let outcome = a.handle_input(ExtractInput::Enter);
        assert_eq!(outcome, Outcome::Copy("second-item-here".to_string()));
    }

    #[test]
    fn enter_copies_the_complete_long_selected_path() {
        let path = "/home/jon/.pi/agent-sessions/2026-09-11/session--long-path";
        let mut a = app(&["short", path]);
        a.handle_input(ExtractInput::Down);

        assert_eq!(
            a.handle_input(ExtractInput::Enter),
            Outcome::Copy(path.into())
        );
    }

    #[test]
    fn enter_copies_a_url_rejoined_across_a_soft_wrap() {
        let text = "Link https://wrap.example/split/path/to/\ncontinued.txt";
        let mut a = ExtractApp::from_visible_text_with_wrap_width(text, Some(40), Theme::default());
        for ch in "wrap.example".chars() {
            a.handle_input(ExtractInput::Char(ch));
        }

        assert_eq!(
            a.handle_input(ExtractInput::Enter),
            Outcome::Copy("https://wrap.example/split/path/to/continued.txt".to_string())
        );
    }

    #[test]
    fn enter_prefers_full_url_and_quote_over_truncated_words() {
        let text = "\
Visit https://example.com/docs/api?v=1 for docs.\n\
single: 'single-quoted-token'\n";
        let mut a = ExtractApp::from_visible_text(text, Theme::default());
        for ch in "example.com".chars() {
            a.handle_input(ExtractInput::Char(ch));
        }
        assert_eq!(
            a.handle_input(ExtractInput::Enter),
            Outcome::Copy("https://example.com/docs/api?v=1".to_string())
        );

        let mut a = ExtractApp::from_visible_text(text, Theme::default());
        for ch in "single-quoted".chars() {
            a.handle_input(ExtractInput::Char(ch));
        }
        assert_eq!(
            a.handle_input(ExtractInput::Enter),
            Outcome::Copy("'single-quoted-token'".to_string())
        );
    }

    #[test]
    fn esc_cancels() {
        let mut a = app(&["only-item-value"]);
        assert_eq!(a.handle_char('\u{1b}'), Outcome::Cancel);
    }

    #[test]
    fn ctrl_c_cancels() {
        let mut a = app(&["only-item-value"]);
        assert_eq!(a.handle_char('\u{3}'), Outcome::Cancel);
    }

    #[test]
    fn enter_with_no_matches_stays() {
        let mut a = app(&["alpha-token"]);
        a.handle_char('z');
        a.handle_char('z');
        assert_eq!(a.filtered_count(), 0);
        assert_eq!(a.handle_input(ExtractInput::Enter), Outcome::Continue);
    }

    #[test]
    fn backspace_widens_filter() {
        let mut a = app(&["alpha-token", "alpine-trail"]);
        a.handle_char('a');
        a.handle_char('l');
        a.handle_char('p');
        a.handle_char('h');
        assert_eq!(a.filtered_count(), 1);
        a.handle_input(ExtractInput::Backspace);
        assert_eq!(a.query(), "alp");
        assert_eq!(a.filtered_count(), 2);
    }

    #[test]
    fn refilter_preserves_selected_item_identity() {
        let mut a = app(&["alpha-token", "beta-selected", "beta-later"]);
        a.handle_input(ExtractInput::Down);
        assert_eq!(a.selected_item().unwrap().text, "beta-selected");

        a.handle_char('b');

        assert_eq!(a.selected_index(), 0);
        assert_eq!(a.selected_item().unwrap().text, "beta-selected");
    }

    #[test]
    fn refilter_selects_first_result_when_selection_disappears() {
        let mut a = app(&["alpha-selected", "beta-first", "beta-second"]);
        assert_eq!(a.selected_item().unwrap().text, "alpha-selected");

        a.handle_char('b');

        assert_eq!(a.selected_index(), 0);
        assert_eq!(a.selected_item().unwrap().text, "beta-first");
    }

    #[test]
    fn no_match_message_survives_typeahead_and_clears_when_matches_return() {
        let mut a = app(&["alpha-token", "beta-token"]);
        assert_eq!(a.message(), None);

        a.handle_char('z');
        assert_eq!(a.filtered_count(), 0);
        assert_eq!(a.message(), Some("no matches"));

        a.handle_input(ExtractInput::Backspace);
        assert_eq!(a.filtered_count(), 2);
        assert_eq!(a.message(), None);

        let empty = ExtractApp::new(Vec::new(), Theme::default());
        assert_eq!(empty.message(), Some("no matches"));
    }

    #[test]
    fn switch_mode_toggles_mode_and_reports_the_target() {
        let mut a = app(&["alpha-token"]);
        assert_eq!(a.mode(), ExtractMode::Scrollback);
        assert_eq!(
            a.handle_char('\t'),
            Outcome::SwitchMode(ExtractMode::Global)
        );
        // The app does not flip its mode until the driver applies the switch,
        // so repeated toggles keep reporting the same target.
        assert_eq!(a.mode(), ExtractMode::Scrollback);

        // After the driver applies the switch, toggling targets the other mode.
        a.apply_mode(ExtractMode::Global, items(&["global-token"]));
        assert_eq!(a.mode(), ExtractMode::Global);
        assert_eq!(
            a.handle_input(ExtractInput::SwitchMode),
            Outcome::SwitchMode(ExtractMode::Scrollback)
        );
    }

    #[test]
    fn fuzzy_matching_is_subsequence_smart_case_and_highlights_positions() {
        let insensitive = fuzzy_match("Herdr Extractor", "hex").unwrap();
        assert_eq!(insensitive.1, vec![0, 6, 7]);

        let smart_case = fuzzy_match("Herdr Extractor", "HX");
        assert!(
            smart_case.is_none(),
            "uppercase query should be case-sensitive"
        );

        let matches = fuzzy_matches(&items(&["latest", "long-token"]), "lt");
        assert!(matches.iter().any(|item| item.index == 1));
        assert_eq!(
            matches
                .iter()
                .find(|item| item.index == 1)
                .unwrap()
                .positions,
            vec![0, 5]
        );
    }

    #[test]
    fn apply_mode_keeps_query_and_refilters() {
        let mut a = app(&["scrollback-only-token", "shared-alpha"]);
        a.handle_char('a');
        assert_eq!(a.filtered_count(), 2);

        a.apply_mode(
            ExtractMode::Global,
            items(&["global-only-gamma", "shared-alpha"]),
        );

        assert_eq!(a.mode(), ExtractMode::Global);
        assert_eq!(a.query(), "a");
        assert_eq!(a.total_count(), 2);
        assert_eq!(a.filtered_count(), 2);
        let texts: Vec<_> = a.visible_rows().iter().map(|(_, text)| *text).collect();
        assert_eq!(texts.len(), 2);
        assert!(texts.contains(&"global-only-gamma"));
        assert!(texts.contains(&"shared-alpha"));
    }

    #[test]
    fn apply_mode_restores_selection_by_item_text() {
        let mut a = app(&["alpha-token", "beta-selected", "gamma-token"]);
        a.handle_input(ExtractInput::Down);
        assert_eq!(a.selected_item().unwrap().text, "beta-selected");

        a.apply_mode(
            ExtractMode::Global,
            items(&["gamma-token", "beta-selected"]),
        );

        assert_eq!(a.selected_item().unwrap().text, "beta-selected");

        // A selection that disappears falls back to the first match.
        a.apply_mode(ExtractMode::Scrollback, items(&["delta-token"]));
        assert_eq!(a.selected_index(), 0);
        assert_eq!(a.selected_item().unwrap().text, "delta-token");
    }

    #[test]
    fn mode_names_and_toggle_round_trip() {
        assert_eq!(ExtractMode::Scrollback.name(), "scrollback");
        assert_eq!(ExtractMode::Global.name(), "global");
        assert_eq!(ExtractMode::Scrollback.toggle(), ExtractMode::Global);
        assert_eq!(ExtractMode::Global.toggle(), ExtractMode::Scrollback);
    }

    #[test]
    fn in_mode_builder_sets_the_initial_mode() {
        let a = ExtractApp::new(items(&["alpha"]), Theme::default()).in_mode(ExtractMode::Global);
        assert_eq!(a.mode(), ExtractMode::Global);
    }

    #[test]
    fn set_message_shows_until_the_next_refilter() {
        let mut a = app(&["alpha-token"]);
        a.set_message(Some("reading global history...".to_string()));
        assert_eq!(a.message(), Some("reading global history..."));
        a.handle_char('a');
        assert_eq!(a.message(), None);
    }

    fn mixed_kind_app() -> ExtractApp {
        ExtractApp::new(
            vec![
                ExtractItem {
                    text: "https://example.com".into(),
                    kind: ItemKind::Url,
                },
                ExtractItem {
                    text: "/tmp/path".into(),
                    kind: ItemKind::Path,
                },
                ExtractItem {
                    text: "plain-word".into(),
                    kind: ItemKind::Word,
                },
                ExtractItem {
                    text: "cargo test".into(),
                    kind: ItemKind::Command,
                },
                ExtractItem {
                    text: "deadbeef".into(),
                    kind: ItemKind::Hash,
                },
            ],
            Theme::default(),
        )
    }

    #[test]
    fn kind_filter_cycles_all_path_url_word_other() {
        let mut a = mixed_kind_app();
        assert_eq!(a.kind_filter(), KindFilter::All);
        assert_eq!(a.filtered_count(), 5);

        a.handle_input(ExtractInput::CycleKindFilter);
        assert_eq!(a.kind_filter(), KindFilter::Path);
        assert_eq!(a.filtered_count(), 1);
        assert_eq!(a.selected_item().unwrap().text, "/tmp/path");

        a.handle_input(ExtractInput::CycleKindFilter);
        assert_eq!(a.kind_filter(), KindFilter::Url);
        assert_eq!(a.selected_item().unwrap().text, "https://example.com");

        a.handle_input(ExtractInput::CycleKindFilter);
        assert_eq!(a.kind_filter(), KindFilter::Word);
        assert_eq!(a.selected_item().unwrap().text, "plain-word");

        a.handle_input(ExtractInput::CycleKindFilter);
        assert_eq!(a.kind_filter(), KindFilter::Other);
        let rows = a.visible_rows();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().any(|(_, text)| *text == "cargo test"));
        assert!(rows.iter().any(|(_, text)| *text == "deadbeef"));

        a.handle_input(ExtractInput::CycleKindFilter);
        assert_eq!(a.kind_filter(), KindFilter::All);
        assert_eq!(a.filtered_count(), 5);
    }

    #[test]
    fn kind_filter_composes_with_typeahead() {
        let mut a = mixed_kind_app();
        a.handle_input(ExtractInput::CycleKindFilter); // path
        a.handle_input(ExtractInput::Char('t'));
        assert_eq!(a.filtered_count(), 1);
        assert_eq!(a.selected_item().unwrap().text, "/tmp/path");

        a.handle_input(ExtractInput::CycleKindFilter); // url — query still "t"
        assert_eq!(a.kind_filter(), KindFilter::Url);
        // "https://example.com" does not fuzzy-match "t" alone? actually 't' matches many
        // Ensure path is gone.
        assert!(a
            .visible_rows()
            .iter()
            .all(|(_, text)| !text.contains("/tmp")));
    }

    #[test]
    fn kind_filter_names_and_cycle_round_trip() {
        assert_eq!(KindFilter::All.name(), "all");
        assert_eq!(KindFilter::Other.name(), "other");
        assert_eq!(
            KindFilter::All.cycle().cycle().cycle().cycle().cycle(),
            KindFilter::All
        );
    }
}
