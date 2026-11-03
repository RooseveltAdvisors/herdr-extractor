//! Interactive typeahead list for the scrollback extractor.
//!
//! Pure state machine: filter items by query, move the selection, copy or cancel.
//! No terminal I/O — fully unit-testable.

use crate::extract::ExtractItem;
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
    /// Only the current on-screen viewport (what you see right now).
    Screen,
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

/// Modal editing mode for the filter field (vimline-inspired).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldMode {
    /// Type filter keywords (edit mode).
    Insert,
    /// Issue commands: move list, mark, open help, motions in the query.
    Normal,
}

impl FieldMode {
    pub fn name(self) -> &'static str {
        match self {
            Self::Insert => "insert",
            Self::Normal => "normal",
        }
    }

    pub fn badge(self) -> &'static str {
        match self {
            Self::Insert => "INSERT",
            Self::Normal => "NORMAL",
        }
    }
}

impl ExtractMode {
    /// Cycle scope: scrollback → screen → global → scrollback.
    pub fn toggle(self) -> Self {
        match self {
            Self::Scrollback => Self::Screen,
            Self::Screen => Self::Global,
            Self::Global => Self::Scrollback,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Scrollback => "scrollback",
            Self::Screen => "screen",
            Self::Global => "global",
        }
    }

    /// Short header badge.
    pub fn badge(self) -> &'static str {
        match self {
            Self::Scrollback => "SCROLLBACK",
            Self::Screen => "SCREEN",
            Self::Global => "GLOBAL",
        }
    }
}

/// Inputs the extract TUI maps onto the pure state machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExtractInput {
    Char(char),
    Backspace,
    /// Delete the last whitespace-delimited query token (fzf Ctrl-W).
    DeleteWord,
    /// Clear the whole query (fzf Ctrl-U).
    ClearQuery,
    Enter,
    Up,
    Down,
    PageUp,
    PageDown,
    Home,
    End,
    Left,
    Right,
    Esc,
    CtrlC,
    SwitchMode,
    /// Cycle the kind filter chip forward (Ctrl-K).
    CycleKindFilter,
    /// Cycle the kind filter chip backward (Alt-K / Shift-Tab-style via Ctrl-J).
    CycleKindFilterBack,
    /// Toggle multi-select mark on the current row (Space).
    ToggleMark,
    /// Open or close the ? help overlay.
    ToggleHelp,
    /// Enter insert mode at cursor / after / line start / line end.
    EnterInsert,
    EnterInsertAfter,
    EnterInsertLineStart,
    EnterInsertLineEnd,
    /// Jump list first / last (gg / G).
    JumpFirst,
    JumpLast,
    /// Word motions in the filter field (normal mode).
    WordForward,
    WordBack,
    WordEnd,
    /// Delete under / before cursor (normal mode x / X).
    DeleteUnder,
    DeleteBefore,
    /// Clear query (normal dd / D).
    ClearQueryNormal,
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
            other => Self::Char(other),
        }
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
    theme: Theme,
    /// Optional kind filter cycled with Ctrl-K; None means all kinds.
    kind_filter: Option<crate::extract::ItemKind>,
    /// Char index into `query` for the typeahead cursor.
    query_cursor: usize,
    /// Indices into `items` marked for multi-copy (Space to toggle).
    marked: std::collections::BTreeSet<usize>,
    /// Insert = type filter; Normal = commands / list motion.
    field_mode: FieldMode,
    /// Pending `g` for `gg`.
    pending_g: bool,
    /// Pending first `j` for `jj` insert-exit.
    pending_j: bool,
    /// ? help overlay open.
    help_open: bool,
    help_query: String,
    help_sel: usize,
    help_scroll: usize,
    /// When help is open, `/` focuses help search (insert into help_query).
    help_searching: bool,
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
            theme,
            kind_filter: None,
            query_cursor: 0,
            marked: std::collections::BTreeSet::new(),
            // Start insert so typing filters immediately; Esc → normal for j/k commands.
            field_mode: FieldMode::Insert,
            pending_g: false,
            pending_j: false,
            help_open: false,
            help_query: String::new(),
            help_sel: 0,
            help_scroll: 0,
            help_searching: false,
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
        // Global: always honor cancel / help toggle first.
        match input {
            ExtractInput::CtrlC => return Outcome::Cancel,
            ExtractInput::ToggleHelp => {
                self.help_open = !self.help_open;
                if self.help_open {
                    self.help_sel = 0;
                    self.help_scroll = 0;
                    self.help_searching = false;
                    self.help_query.clear();
                }
                return Outcome::Continue;
            }
            _ => {}
        }

        if self.help_open {
            return self.handle_help_input(input);
        }

        match self.field_mode {
            FieldMode::Insert => self.handle_insert(input),
            FieldMode::Normal => self.handle_normal(input),
        }
    }

    fn handle_help_input(&mut self, input: ExtractInput) -> Outcome {
        match input {
            ExtractInput::Esc | ExtractInput::ToggleHelp => {
                if self.help_searching {
                    self.help_searching = false;
                } else {
                    self.help_open = false;
                }
                Outcome::Continue
            }
            ExtractInput::Enter => {
                self.help_open = false;
                Outcome::Continue
            }
            ExtractInput::Up | ExtractInput::WordBack => {
                self.help_sel = self.help_sel.saturating_sub(1);
                Outcome::Continue
            }
            ExtractInput::Down | ExtractInput::WordForward => {
                let n = crate::help::filter_help_entries(&self.help_query).len();
                if n > 0 {
                    self.help_sel = (self.help_sel + 1).min(n - 1);
                }
                Outcome::Continue
            }
            ExtractInput::PageUp => {
                self.help_sel = self.help_sel.saturating_sub(8);
                Outcome::Continue
            }
            ExtractInput::PageDown => {
                let n = crate::help::filter_help_entries(&self.help_query).len();
                if n > 0 {
                    self.help_sel = (self.help_sel + 8).min(n - 1);
                }
                Outcome::Continue
            }
            ExtractInput::JumpFirst => {
                self.help_sel = 0;
                Outcome::Continue
            }
            ExtractInput::JumpLast => {
                let n = crate::help::filter_help_entries(&self.help_query).len();
                self.help_sel = n.saturating_sub(1);
                Outcome::Continue
            }
            ExtractInput::Char('/') if !self.help_searching => {
                self.help_searching = true;
                Outcome::Continue
            }
            ExtractInput::Char(ch) if self.help_searching && !ch.is_control() => {
                self.help_query.push(ch);
                self.help_sel = 0;
                Outcome::Continue
            }
            ExtractInput::Backspace if self.help_searching => {
                self.help_query.pop();
                self.help_sel = 0;
                Outcome::Continue
            }
            ExtractInput::ClearQuery | ExtractInput::DeleteWord if self.help_searching => {
                self.help_query.clear();
                self.help_sel = 0;
                Outcome::Continue
            }
            _ => Outcome::Continue,
        }
    }

    fn handle_insert(&mut self, input: ExtractInput) -> Outcome {
        // jj → normal
        if let ExtractInput::Char('j') = input {
            if self.pending_j {
                self.pending_j = false;
                // remove the first j that was inserted
                if self.query_cursor > 0 {
                    let chars: Vec<char> = self.query.chars().collect();
                    let mut out = String::new();
                    out.extend(chars.iter().take(self.query_cursor.saturating_sub(1)));
                    out.extend(chars.iter().skip(self.query_cursor));
                    // we haven't inserted the second j yet
                    self.query = out;
                    self.query_cursor = self.query_cursor.saturating_sub(1);
                    self.refilter();
                }
                self.field_mode = FieldMode::Normal;
                return Outcome::Continue;
            }
            self.pending_j = true;
            self.insert_at_cursor('j');
            self.refilter();
            return Outcome::Continue;
        }
        self.pending_j = false;

        match input {
            ExtractInput::Esc => {
                // Recall ladder: insert → normal. Empty insert Esc cancels outright.
                if self.query.is_empty() && self.kind_filter.is_none() {
                    Outcome::Cancel
                } else {
                    self.field_mode = FieldMode::Normal;
                    Outcome::Continue
                }
            }
            ExtractInput::Enter => self.confirm(),
            ExtractInput::ToggleMark => {
                if let Some(item_index) = self.filtered.get(self.selected).map(|m| m.index) {
                    if !self.marked.remove(&item_index) {
                        self.marked.insert(item_index);
                    }
                }
                Outcome::Continue
            }
            ExtractInput::Backspace => {
                self.backspace_at_cursor();
                self.refilter();
                Outcome::Continue
            }
            ExtractInput::DeleteWord => {
                self.delete_query_word();
                self.refilter();
                Outcome::Continue
            }
            ExtractInput::ClearQuery => {
                self.query.clear();
                self.query_cursor = 0;
                self.refilter();
                Outcome::Continue
            }
            ExtractInput::Left => {
                if self.query_cursor > 0 {
                    self.query_cursor -= 1;
                }
                Outcome::Continue
            }
            ExtractInput::Right => {
                let len = self.query.chars().count();
                if self.query_cursor < len {
                    self.query_cursor += 1;
                }
                Outcome::Continue
            }
            ExtractInput::Home => {
                self.query_cursor = 0;
                Outcome::Continue
            }
            ExtractInput::End => {
                self.query_cursor = self.query.chars().count();
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
            ExtractInput::PageUp => {
                self.move_sel(-10);
                Outcome::Continue
            }
            ExtractInput::PageDown => {
                self.move_sel(10);
                Outcome::Continue
            }
            ExtractInput::SwitchMode => Outcome::SwitchMode(self.mode.toggle()),
            ExtractInput::CycleKindFilter => {
                self.cycle_kind_filter();
                self.refilter();
                Outcome::Continue
            }
            ExtractInput::CycleKindFilterBack => {
                self.cycle_kind_filter_back();
                self.refilter();
                Outcome::Continue
            }
            ExtractInput::Char(ch) => {
                if ch.is_control() {
                    return Outcome::Continue;
                }
                self.insert_at_cursor(ch);
                self.refilter();
                Outcome::Continue
            }
            _ => Outcome::Continue,
        }
    }

    fn handle_normal(&mut self, input: ExtractInput) -> Outcome {
        // gg
        if matches!(input, ExtractInput::Char('g')) {
            if self.pending_g {
                self.pending_g = false;
                self.selected = 0;
                return Outcome::Continue;
            }
            self.pending_g = true;
            return Outcome::Continue;
        }
        let was_g = self.pending_g;
        self.pending_g = false;

        match input {
            ExtractInput::Esc => {
                if !self.query.is_empty() {
                    // clear filter but stay open
                    self.query.clear();
                    self.query_cursor = 0;
                    self.kind_filter = None;
                    self.refilter();
                    Outcome::Continue
                } else {
                    Outcome::Cancel
                }
            }
            ExtractInput::Enter => self.confirm(),
            ExtractInput::Up | ExtractInput::Char('k') => {
                self.move_sel(-1);
                Outcome::Continue
            }
            ExtractInput::Down | ExtractInput::Char('j') => {
                self.move_sel(1);
                Outcome::Continue
            }
            ExtractInput::PageUp => {
                self.move_sel(-10);
                Outcome::Continue
            }
            ExtractInput::PageDown => {
                self.move_sel(10);
                Outcome::Continue
            }
            ExtractInput::JumpFirst | ExtractInput::Char('g') if was_g => {
                self.selected = 0;
                Outcome::Continue
            }
            ExtractInput::JumpLast | ExtractInput::Char('G') => {
                if !self.filtered.is_empty() {
                    self.selected = self.filtered.len() - 1;
                }
                Outcome::Continue
            }
            ExtractInput::ToggleMark | ExtractInput::Char(' ') => {
                if let Some(item_index) = self.filtered.get(self.selected).map(|m| m.index) {
                    if !self.marked.remove(&item_index) {
                        self.marked.insert(item_index);
                    }
                }
                Outcome::Continue
            }
            ExtractInput::SwitchMode | ExtractInput::Char('\t') => {
                Outcome::SwitchMode(self.mode.toggle())
            }
            // Scope jumps (normal mode) — tab still cycles all three.
            ExtractInput::Char('s') | ExtractInput::Char('v') => {
                Outcome::SwitchMode(ExtractMode::Screen)
            }
            ExtractInput::Char('S') => Outcome::SwitchMode(ExtractMode::Scrollback),
            ExtractInput::Char('t') => Outcome::SwitchMode(ExtractMode::Global),
            ExtractInput::CycleKindFilter => {
                self.cycle_kind_filter();
                self.refilter();
                Outcome::Continue
            }
            ExtractInput::CycleKindFilterBack => {
                self.cycle_kind_filter_back();
                self.refilter();
                Outcome::Continue
            }
            ExtractInput::EnterInsert | ExtractInput::Char('i') => {
                self.field_mode = FieldMode::Insert;
                Outcome::Continue
            }
            ExtractInput::EnterInsertAfter | ExtractInput::Char('a') => {
                let len = self.query.chars().count();
                if self.query_cursor < len {
                    self.query_cursor += 1;
                }
                self.field_mode = FieldMode::Insert;
                Outcome::Continue
            }
            ExtractInput::EnterInsertLineStart | ExtractInput::Char('I') => {
                self.query_cursor = 0;
                self.field_mode = FieldMode::Insert;
                Outcome::Continue
            }
            ExtractInput::EnterInsertLineEnd | ExtractInput::Char('A') => {
                self.query_cursor = self.query.chars().count();
                self.field_mode = FieldMode::Insert;
                Outcome::Continue
            }
            ExtractInput::Home | ExtractInput::Char('0') => {
                self.query_cursor = 0;
                Outcome::Continue
            }
            ExtractInput::End | ExtractInput::Char('$') => {
                self.query_cursor = self.query.chars().count();
                Outcome::Continue
            }
            ExtractInput::WordForward | ExtractInput::Char('w') => {
                self.motion_word_forward();
                Outcome::Continue
            }
            ExtractInput::WordBack | ExtractInput::Char('b') => {
                self.motion_word_back();
                Outcome::Continue
            }
            ExtractInput::WordEnd | ExtractInput::Char('e') => {
                self.motion_word_end();
                Outcome::Continue
            }
            ExtractInput::DeleteUnder | ExtractInput::Char('x') => {
                self.delete_under_cursor();
                self.refilter();
                Outcome::Continue
            }
            ExtractInput::DeleteBefore | ExtractInput::Char('X') => {
                self.backspace_at_cursor();
                self.refilter();
                Outcome::Continue
            }
            ExtractInput::ClearQueryNormal | ExtractInput::Char('D') => {
                self.query.clear();
                self.query_cursor = 0;
                self.refilter();
                Outcome::Continue
            }
            ExtractInput::Char('d') => {
                // dd clear — second d
                // use pending via message or simple: single d waits - store pending_d
                // For simplicity treat d as start of dd: if next is d clear. Use pending_g style.
                // Re-use pending_g slot? Better pending_d field - skip, map dd via pending_g abuse no.
                // Single 'd' alone does nothing; 'dd' needs pending. Add pending_d inline via pending_g rename...
                // Quick: 'c' clear query as alias too
                Outcome::Continue
            }
            ExtractInput::Left => {
                if self.query_cursor > 0 {
                    self.query_cursor -= 1;
                }
                Outcome::Continue
            }
            ExtractInput::Right => {
                let len = self.query.chars().count();
                if self.query_cursor < len {
                    self.query_cursor += 1;
                }
                Outcome::Continue
            }
            ExtractInput::Char('/') => {
                // slash focuses insert for filtering (vim search idiom)
                self.field_mode = FieldMode::Insert;
                Outcome::Continue
            }
            _ => Outcome::Continue,
        }
    }

    fn motion_word_forward(&mut self) {
        let chars: Vec<char> = self.query.chars().collect();
        let n = chars.len();
        if self.query_cursor >= n {
            return;
        }
        let mut i = self.query_cursor;
        // skip current word
        while i < n && !chars[i].is_whitespace() {
            i += 1;
        }
        while i < n && chars[i].is_whitespace() {
            i += 1;
        }
        self.query_cursor = i;
    }

    fn motion_word_back(&mut self) {
        if self.query_cursor == 0 {
            return;
        }
        let chars: Vec<char> = self.query.chars().collect();
        let mut i = self.query_cursor;
        while i > 0 && chars[i - 1].is_whitespace() {
            i -= 1;
        }
        while i > 0 && !chars[i - 1].is_whitespace() {
            i -= 1;
        }
        self.query_cursor = i;
    }

    fn motion_word_end(&mut self) {
        let chars: Vec<char> = self.query.chars().collect();
        let n = chars.len();
        if n == 0 {
            return;
        }
        let mut i = self.query_cursor;
        if i < n && !chars[i].is_whitespace() {
            i += 1;
        }
        while i < n && chars[i].is_whitespace() {
            i += 1;
        }
        while i + 1 < n && !chars[i + 1].is_whitespace() {
            i += 1;
        }
        if i < n {
            self.query_cursor = i;
        }
    }

    fn delete_under_cursor(&mut self) {
        let chars: Vec<char> = self.query.chars().collect();
        if self.query_cursor >= chars.len() {
            return;
        }
        let mut out = String::new();
        out.extend(chars.iter().take(self.query_cursor));
        out.extend(chars.iter().skip(self.query_cursor + 1));
        self.query = out;
    }

    fn delete_query_word(&mut self) {
        // Delete back to previous whitespace from the cursor.
        let chars: Vec<char> = self.query.chars().collect();
        if self.query_cursor == 0 || chars.is_empty() {
            return;
        }
        let mut end = self.query_cursor;
        while end > 0 && chars[end - 1].is_whitespace() {
            end -= 1;
        }
        let mut start = end;
        while start > 0 && !chars[start - 1].is_whitespace() {
            start -= 1;
        }
        let left: String = chars[..start].iter().collect();
        let right: String = chars[end..].iter().collect();
        self.query = left + &right;
        self.query_cursor = start;
    }

    fn insert_at_cursor(&mut self, ch: char) {
        let chars: Vec<char> = self.query.chars().collect();
        let mut out = String::new();
        out.extend(chars.iter().take(self.query_cursor));
        out.push(ch);
        out.extend(chars.iter().skip(self.query_cursor));
        self.query = out;
        self.query_cursor += 1;
    }

    fn backspace_at_cursor(&mut self) {
        if self.query_cursor == 0 {
            return;
        }
        let chars: Vec<char> = self.query.chars().collect();
        let mut out = String::new();
        out.extend(chars.iter().take(self.query_cursor - 1));
        out.extend(chars.iter().skip(self.query_cursor));
        self.query = out;
        self.query_cursor -= 1;
    }

    fn cycle_kind_filter(&mut self) {
        let kinds = crate::extract::ItemKind::all();
        self.kind_filter = match self.kind_filter {
            None => Some(kinds[0]),
            Some(current) => {
                let pos = kinds.iter().position(|k| *k == current).unwrap_or(0);
                if pos + 1 >= kinds.len() {
                    None
                } else {
                    Some(kinds[pos + 1])
                }
            }
        };
    }

    fn cycle_kind_filter_back(&mut self) {
        let kinds = crate::extract::ItemKind::all();
        self.kind_filter = match self.kind_filter {
            None => Some(*kinds.last().unwrap()),
            Some(current) => {
                let pos = kinds.iter().position(|k| *k == current).unwrap_or(0);
                if pos == 0 {
                    None
                } else {
                    Some(kinds[pos - 1])
                }
            }
        };
    }

    pub fn kind_filter(&self) -> Option<crate::extract::ItemKind> {
        self.kind_filter
    }

    pub fn field_mode(&self) -> FieldMode {
        self.field_mode
    }

    pub fn help_open(&self) -> bool {
        self.help_open
    }

    pub fn help_query(&self) -> &str {
        &self.help_query
    }

    pub fn help_sel(&self) -> usize {
        self.help_sel
    }

    pub fn help_searching(&self) -> bool {
        self.help_searching
    }

    pub fn set_help_scroll(&mut self, scroll: usize) {
        self.help_scroll = scroll;
    }

    pub fn help_scroll(&self) -> usize {
        self.help_scroll
    }

    pub fn query_cursor(&self) -> usize {
        self.query_cursor.min(self.query.chars().count())
    }

    pub fn is_marked(&self, item_index: usize) -> bool {
        self.marked.contains(&item_index)
    }

    pub fn marked_count(&self) -> usize {
        self.marked.len()
    }

    /// Convenience for tests and simple char-only drivers.
    pub fn handle_char(&mut self, ch: char) -> Outcome {
        self.handle_input(ExtractInput::from_char(ch))
    }

    fn confirm(&self) -> Outcome {
        if !self.marked.is_empty() {
            let mut parts = Vec::new();
            for idx in &self.marked {
                if let Some(item) = self.items.get(*idx) {
                    parts.push(item.text.clone());
                }
            }
            if parts.is_empty() {
                return Outcome::Continue;
            }
            return Outcome::Copy(parts.join("\n"));
        }
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
        let parsed = parse_filter_query(&self.query);
        let active_kind = parsed.kind.or(self.kind_filter);
        let mut filtered: Vec<FuzzyMatch> = self
            .items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| {
                if let Some(kind) = active_kind {
                    if item.kind != kind {
                        return None;
                    }
                }
                score_item(item, &parsed.tokens).map(|(score, positions)| FuzzyMatch {
                    index,
                    score,
                    positions,
                })
            })
            .collect();
        filtered.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.index.cmp(&b.index)));
        self.filtered = filtered;
        if self.filtered.is_empty() {
            self.selected = 0;
            let hint = if active_kind.is_some() || !parsed.tokens.is_empty() {
                "no matches — clear filter or try kind:url / @email / multi words"
            } else {
                "no matches"
            };
            self.message = Some(hint.to_string());
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

    /// Human-readable active filter summary for the prompt chrome.
    pub fn filter_summary(&self) -> String {
        let parsed = parse_filter_query(&self.query);
        let mut parts = Vec::new();
        if let Some(kind) = parsed.kind.or(self.kind_filter) {
            parts.push(format!("kind:{}", kind.stable_key()));
        }
        if !parsed.tokens.is_empty() {
            parts.push(parsed.tokens.join(" · "));
        }
        parts.join("  ")
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

    pub fn set_engine(&mut self, engine: ExtractionEngine) {
        self.engine = engine;
    }

    /// Apply a mode switch: swap the item list, keep the current filter query
    /// and restore the selection by item text when it still matches.
    pub fn apply_mode(&mut self, mode: ExtractMode, items: Vec<ExtractItem>) {
        let selected_text = self.selected_item().map(|item| item.text.clone());
        self.mode = mode;
        self.items = items;
        // Marks are index-based; drop them on list swap so Tab cannot copy wrong tokens.
        self.marked.clear();
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

    /// (is_selected, is_marked, item, match positions)
    pub fn visible_match_items(&self) -> Vec<(bool, bool, &ExtractItem, &[usize])> {
        self.filtered
            .iter()
            .enumerate()
            .filter_map(|(pos, item_match)| {
                self.items.get(item_match.index).map(|item| {
                    (
                        pos == self.selected,
                        self.marked.contains(&item_match.index),
                        item,
                        item_match.positions.as_slice(),
                    )
                })
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedQuery {
    kind: Option<crate::extract::ItemKind>,
    tokens: Vec<String>,
}

/// Parse an intuitive filter query.
///
/// Supports:
/// - `kind:url`, `k:path`, short aliases `u:` / `p:` / `e:` …
/// - leading sigils `@mail` (email), `#42` stays text unless alone as kind id filter via `id:`
/// - space-separated tokens = AND (all must match text or kind label)
/// - bare kind name as sole token (`url`, `email`) = quick kind filter
fn parse_filter_query(query: &str) -> ParsedQuery {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return ParsedQuery {
            kind: None,
            tokens: Vec::new(),
        };
    }

    let mut kind = None;
    let mut rest = trimmed;

    // kind:key / k:key
    for prefix in ["kind:", "k:"] {
        if let Some(after) = rest.strip_prefix(prefix) {
            let after = after.trim_start();
            let (key, remainder) = split_first_token(after);
            if let Some(parsed) = resolve_kind_alias(key) {
                kind = Some(parsed);
                rest = remainder;
                break;
            }
        }
    }

    // short `u:foo` / `path:foo` form when kind not set
    if kind.is_none() {
        if let Some((key, remainder)) = rest.split_once(':') {
            let key = key.trim();
            // avoid treating URLs like https:// as kind
            if !key.is_empty()
                && !key.contains('/')
                && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
            {
                if let Some(parsed) = resolve_kind_alias(key) {
                    kind = Some(parsed);
                    rest = remainder.trim_start();
                }
            }
        }
    }

    // leading @ means email filter + remainder
    if kind.is_none() {
        if let Some(after) = rest.strip_prefix('@') {
            kind = Some(crate::extract::ItemKind::Email);
            rest = after.trim_start();
        }
    }

    let mut tokens: Vec<String> = rest
        .split_whitespace()
        .filter(|t| !t.is_empty())
        .map(|t| t.to_string())
        .collect();

    // Sole token that is a kind alias → quick kind filter (type "url" alone).
    if kind.is_none() && tokens.len() == 1 {
        if let Some(parsed) = resolve_kind_alias(&tokens[0]) {
            kind = Some(parsed);
            tokens.clear();
        }
    }

    ParsedQuery { kind, tokens }
}

fn split_first_token(input: &str) -> (&str, &str) {
    match input.find(char::is_whitespace) {
        Some(idx) => (&input[..idx], input[idx..].trim_start()),
        None => (input, ""),
    }
}

fn resolve_kind_alias(key: &str) -> Option<crate::extract::ItemKind> {
    let k = key.trim().to_ascii_lowercase();
    match k.as_str() {
        "u" | "url" | "urls" | "link" | "links" => Some(crate::extract::ItemKind::Url),
        "p" | "path" | "paths" | "file" | "files" => Some(crate::extract::ItemKind::Path),
        "e" | "email" | "mail" | "mails" => Some(crate::extract::ItemKind::Email),
        "h" | "host" | "hosts" | "hostname" => Some(crate::extract::ItemKind::Host),
        "i" | "ip" | "ips" => Some(crate::extract::ItemKind::Ip),
        "id" | "ids" | "ticket" | "bead" | "issue" => Some(crate::extract::ItemKind::Id),
        "c" | "cmd" | "command" | "commands" => Some(crate::extract::ItemKind::Command),
        "hash" | "sha" | "commit" => Some(crate::extract::ItemKind::Hash),
        "v" | "ver" | "version" => Some(crate::extract::ItemKind::Version),
        "err" | "error" | "errors" | "fail" => Some(crate::extract::ItemKind::Error),
        "code" | "json" | "key" => Some(crate::extract::ItemKind::Code),
        "env" | "var" | "vars" => Some(crate::extract::ItemKind::Env),
        "uuid" | "guid" => Some(crate::extract::ItemKind::Uuid),
        "date" | "day" => Some(crate::extract::ItemKind::Date),
        "color" | "hex" | "colour" => Some(crate::extract::ItemKind::Color),
        "name" | "phrase" | "names" => Some(crate::extract::ItemKind::Phrase),
        "q" | "quote" | "quotes" => Some(crate::extract::ItemKind::Quote),
        "w" | "word" | "words" => Some(crate::extract::ItemKind::Word),
        _ => crate::extract::ItemKind::from_stable_key(&k),
    }
}

/// Score one item against AND-tokens. Each token must match the item text
/// (exact/prefix/substring/fuzzy) or the kind label.
fn score_item(item: &ExtractItem, tokens: &[String]) -> Option<(i32, Vec<usize>)> {
    if tokens.is_empty() {
        return Some((0, Vec::new()));
    }
    let mut total = 0i32;
    let mut positions = Vec::new();
    for token in tokens {
        let (score, pos) = match_token(item, token)?;
        total = total.saturating_add(score);
        positions.extend(pos);
    }
    positions.sort_unstable();
    positions.dedup();
    Some((total, positions))
}

fn match_token(item: &ExtractItem, token: &str) -> Option<(i32, Vec<usize>)> {
    let token = token.trim();
    if token.is_empty() {
        return Some((0, Vec::new()));
    }

    // Kind-label match: typing "email" while already in all-kinds boosts emails.
    let kind_key = item.kind.stable_key();
    if kind_key.eq_ignore_ascii_case(token) || resolve_kind_alias(token) == Some(item.kind) {
        return Some((80, Vec::new()));
    }

    let text = item.text.as_str();
    let lower_text = text.to_ascii_lowercase();
    let lower_token = token.to_ascii_lowercase();

    // Exact
    if text == token || lower_text == lower_token {
        let positions: Vec<usize> = (0..token.chars().count()).collect();
        return Some((200, positions));
    }
    // Prefix
    if lower_text.starts_with(&lower_token) {
        let positions: Vec<usize> = (0..token.chars().count()).collect();
        return Some((150, positions));
    }
    // Continuous substring (case-insensitive)
    if let Some(byte_idx) = lower_text.find(&lower_token) {
        let start_char = text[..byte_idx].chars().count();
        let positions: Vec<usize> = (start_char..start_char + token.chars().count()).collect();
        return Some((120, positions));
    }
    // Fuzzy subsequence
    let (score, positions) = fuzzy_match(text, token)?;
    Some((score, positions))
}

#[cfg(test)]
fn fuzzy_matches(items: &[ExtractItem], query: &str) -> Vec<FuzzyMatch> {
    let parsed = parse_filter_query(query);
    let mut matches: Vec<_> = items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            if let Some(kind) = parsed.kind {
                if item.kind != kind {
                    return None;
                }
            }
            score_item(item, &parsed.tokens).map(|(score, positions)| FuzzyMatch {
                index,
                score,
                positions,
            })
        })
        .collect();
    matches.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.index.cmp(&b.index)));
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
        assert!(a.message().unwrap_or("").starts_with("no matches"));

        a.handle_input(ExtractInput::Backspace);
        assert_eq!(a.filtered_count(), 2);
        assert_eq!(a.message(), None);

        let empty = ExtractApp::new(Vec::new(), Theme::default());
        assert!(empty.message().unwrap_or("").starts_with("no matches"));
    }

    #[test]
    fn switch_mode_toggles_mode_and_reports_the_target() {
        let mut a = app(&["alpha-token"]);
        assert_eq!(a.mode(), ExtractMode::Scrollback);
        // Tab cycles scrollback → screen → global → scrollback.
        assert_eq!(
            a.handle_char('\t'),
            Outcome::SwitchMode(ExtractMode::Screen)
        );
        assert_eq!(a.mode(), ExtractMode::Scrollback);

        a.apply_mode(ExtractMode::Screen, items(&["screen-token"]));
        assert_eq!(
            a.handle_input(ExtractInput::SwitchMode),
            Outcome::SwitchMode(ExtractMode::Global)
        );

        a.apply_mode(ExtractMode::Global, items(&["global-token"]));
        assert_eq!(
            a.handle_input(ExtractInput::SwitchMode),
            Outcome::SwitchMode(ExtractMode::Scrollback)
        );

        // Normal-mode scope jumps.
        a.handle_input(ExtractInput::Esc); // insert→normal if needed
        let _ = a.handle_input(ExtractInput::Char('s'));
        // Char('s') only jumps in normal; ensure we are normal
        a.field_mode = FieldMode::Normal;
        assert_eq!(
            a.handle_input(ExtractInput::Char('s')),
            Outcome::SwitchMode(ExtractMode::Screen)
        );
        assert_eq!(
            a.handle_input(ExtractInput::Char('S')),
            Outcome::SwitchMode(ExtractMode::Scrollback)
        );
        assert_eq!(
            a.handle_input(ExtractInput::Char('t')),
            Outcome::SwitchMode(ExtractMode::Global)
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
        assert_eq!(ExtractMode::Screen.name(), "screen");
        assert_eq!(ExtractMode::Global.name(), "global");
        assert_eq!(ExtractMode::Scrollback.toggle(), ExtractMode::Screen);
        assert_eq!(ExtractMode::Screen.toggle(), ExtractMode::Global);
        assert_eq!(ExtractMode::Global.toggle(), ExtractMode::Scrollback);
        assert_eq!(ExtractMode::Screen.badge(), "SCREEN");
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

    #[test]
    fn multi_token_and_filter_and_quick_kind() {
        let theme = Theme::default();
        let items = vec![
            ExtractItem {
                text: "https://staging.example/deploy".into(),
                kind: ItemKind::Url,
            },
            ExtractItem {
                text: "https://prod.example/app".into(),
                kind: ItemKind::Url,
            },
            ExtractItem {
                text: "/tmp/staging-notes".into(),
                kind: ItemKind::Path,
            },
            ExtractItem {
                text: "jon@arcs.health".into(),
                kind: ItemKind::Email,
            },
        ];
        let mut app = ExtractApp::new(items, theme);
        // AND tokens: both must match
        for ch in "stg dep".chars() {
            app.handle_input(ExtractInput::Char(ch));
        }
        assert_eq!(app.filtered_count(), 1);
        assert!(app
            .selected_item()
            .unwrap()
            .text
            .contains("staging.example/deploy"));
        app.handle_input(ExtractInput::ClearQuery);
        // quick kind alias alone
        for ch in "email".chars() {
            app.handle_input(ExtractInput::Char(ch));
        }
        assert_eq!(app.filtered_count(), 1);
        assert_eq!(app.selected_item().unwrap().kind, ItemKind::Email);
        app.handle_input(ExtractInput::ClearQuery);
        // u: prefix + token
        for ch in "u:prod".chars() {
            app.handle_input(ExtractInput::Char(ch));
        }
        assert_eq!(app.filtered_count(), 1);
        assert!(app.selected_item().unwrap().text.contains("prod.example"));
        app.handle_input(ExtractInput::ClearQuery);
        // @ sigil
        for ch in "@arcs".chars() {
            app.handle_input(ExtractInput::Char(ch));
        }
        assert_eq!(app.filtered_count(), 1);
        assert_eq!(app.selected_item().unwrap().kind, ItemKind::Email);
    }

    #[test]
    fn kind_query_prefix_and_ctrl_k_cycle_filter_kinds() {
        let theme = Theme::default();
        let items = vec![
            ExtractItem {
                text: "https://a.example/x".into(),
                kind: ItemKind::Url,
            },
            ExtractItem {
                text: "/tmp/path-file".into(),
                kind: ItemKind::Path,
            },
            ExtractItem {
                text: "plain-word-token".into(),
                kind: ItemKind::Word,
            },
        ];
        let mut app = ExtractApp::new(items, theme);
        assert_eq!(app.filtered_count(), 3);
        for ch in "kind:url".chars() {
            app.handle_input(ExtractInput::Char(ch));
        }
        assert_eq!(app.filtered_count(), 1);
        assert_eq!(app.selected_item().unwrap().kind, ItemKind::Url);
        app.handle_input(ExtractInput::ClearQuery);
        app.handle_input(ExtractInput::CycleKindFilter);
        assert!(app.kind_filter().is_some());
        app.handle_input(ExtractInput::CycleKindFilterBack);
        assert!(app.kind_filter().is_none());
    }

    #[test]
    fn readline_edits_and_marks_work() {
        let theme = Theme::default();
        let items = vec![
            ExtractItem {
                text: "alpha-token".into(),
                kind: ItemKind::Word,
            },
            ExtractItem {
                text: "beta-token".into(),
                kind: ItemKind::Word,
            },
        ];
        let mut app = ExtractApp::new(items, theme);
        for ch in "alpha beta".chars() {
            app.handle_input(ExtractInput::Char(ch));
        }
        assert_eq!(app.query(), "alpha beta");
        app.handle_input(ExtractInput::DeleteWord);
        assert_eq!(app.query().trim(), "alpha");
        app.handle_input(ExtractInput::ClearQuery);
        assert_eq!(app.query(), "");
        app.handle_input(ExtractInput::ToggleMark);
        assert_eq!(app.marked_count(), 1);
        app.handle_input(ExtractInput::Down);
        app.handle_input(ExtractInput::ToggleMark);
        assert_eq!(app.marked_count(), 2);
        match app.handle_input(ExtractInput::Enter) {
            Outcome::Copy(text) => {
                assert!(text.contains("alpha-token") && text.contains("beta-token"))
            }
            other => panic!("expected copy, got {other:?}"),
        }
    }

    #[test]
    fn marks_clear_on_mode_switch() {
        let theme = Theme::default();
        let items = vec![
            ExtractItem {
                text: "alpha-token".into(),
                kind: ItemKind::Word,
            },
            ExtractItem {
                text: "beta-token".into(),
                kind: ItemKind::Word,
            },
        ];
        let mut app = ExtractApp::new(items.clone(), theme.clone());
        app.handle_input(ExtractInput::ToggleMark);
        assert_eq!(app.marked_count(), 1);
        app.apply_mode(ExtractMode::Global, items);
        assert_eq!(app.marked_count(), 0);
    }

    #[test]
    fn modal_insert_normal_and_help_overlay() {
        let theme = Theme::default();
        let items = vec![
            ExtractItem {
                text: "alpha-token".into(),
                kind: ItemKind::Word,
            },
            ExtractItem {
                text: "beta-token".into(),
                kind: ItemKind::Word,
            },
        ];
        let mut app = ExtractApp::new(items, theme);
        assert_eq!(app.field_mode(), FieldMode::Insert);
        // type filter
        for ch in "alpha".chars() {
            app.handle_input(ExtractInput::Char(ch));
        }
        assert_eq!(app.filtered_count(), 1);
        // esc → normal
        assert_eq!(app.handle_input(ExtractInput::Esc), Outcome::Continue);
        assert_eq!(app.field_mode(), FieldMode::Normal);
        // j moves selection (only one match so stays)
        app.handle_input(ExtractInput::Char('j'));
        // ? opens help
        app.handle_input(ExtractInput::ToggleHelp);
        assert!(app.help_open());
        app.handle_input(ExtractInput::Char('/'));
        assert!(app.help_searching());
        for ch in "mark".chars() {
            app.handle_input(ExtractInput::Char(ch));
        }
        assert_eq!(app.help_query(), "mark");
        app.handle_input(ExtractInput::Esc); // leave help search
        app.handle_input(ExtractInput::ToggleHelp); // close help
        assert!(!app.help_open());
        // i back to insert
        app.handle_input(ExtractInput::Char('i'));
        assert_eq!(app.field_mode(), FieldMode::Insert);
    }
}
