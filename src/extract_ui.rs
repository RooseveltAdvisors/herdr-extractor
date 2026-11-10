//! Ratatui rendering for the extract typeahead list.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use ratatui::Frame;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::extract::{ExtractItem, ItemKind};
use crate::extract_app::{ExtractApp, ExtractionEngine};
use crate::herdr_client::PaneGeometry;

/// Pane rows consumed below the last client-visible row by Herdr's floating
/// overlay chrome. Measured against a live 0.9.0 rendered capture; keep this
/// tunable if Herdr changes the border/padding height.
pub const RESERVED_BOTTOM_ROWS: u16 = 3;

pub fn draw(frame: &mut Frame<'_>, app: &ExtractApp) {
    draw_with_visible_geometry(frame, app, None);
}

/// Draw across the picker PTY. Herdr's reported pane rectangle may be narrower
/// than the overlay it paints, so it can only constrain height.
pub fn draw_with_visible_geometry(
    frame: &mut Frame<'_>,
    app: &ExtractApp,
    geometry: Option<PaneGeometry>,
) {
    let area = drawable_area(frame.area(), geometry);
    if area.height == 0 || area.width == 0 {
        return;
    }
    if app.help_open() {
        draw_help(frame, app, area);
        return;
    }
    draw_header(frame, app, area);
    draw_prompt(frame, app, area);
    let detail_lines = selected_detail_lines(app, usize::from(area.width));
    let detail_height = detail_lines
        .len()
        .min(usize::from(detail_rows_available(area))) as u16;

    let body_area = drawable_body_area_with_detail(area, detail_height);
    let lines = render_body(app, usize::from(body_area.height), usize::from(area.width));
    frame.render_widget(Paragraph::new(lines), body_area);

    // Render detail preview at the bottom of the window (above reserved chrome),
    // so the matches list begins immediately below the prompt on row 2.
    if detail_height > 0 {
        let detail_area = Rect {
            x: area.x,
            y: body_area.y + body_area.height,
            width: area.width,
            height: detail_height,
        };
        draw_detail(frame, app, detail_area, &detail_lines);
    }
    if let Some(reserved_area) = reserved_bottom_rows(area) {
        frame.render_widget(Clear, reserved_area);
    }

    // In Normal mode, place the hardware cursor on the active match row
    // (on the ▌ selection rail), so the cursor rests directly on the match.
    if area.height >= 2 && app.field_mode() == crate::extract_app::FieldMode::Normal {
        let selected = app.selected_index();
        let rows_len = app.filtered_count();
        if rows_len > 0 && body_area.height > 0 {
            let max_rows = usize::from(body_area.height);
            let start = if rows_len <= max_rows || selected < max_rows / 2 {
                0
            } else if selected + (max_rows - max_rows / 2) >= rows_len {
                rows_len - max_rows
            } else {
                selected - max_rows / 2
            };
            let row_offset = selected.saturating_sub(start) as u16;
            if row_offset < body_area.height {
                frame.set_cursor_position((body_area.x, body_area.y + row_offset));
            }
        }
    }
}

fn draw_help(frame: &mut Frame<'_>, app: &ExtractApp, area: Rect) {
    let theme = app.theme();
    let width = usize::from(area.width);
    let height = usize::from(reserved_bottom_rows(area).map_or(area.height, |r| r.y - area.y));
    if height == 0 || width == 0 {
        return;
    }

    let entries = crate::help::filter_help_entries(app.help_query());
    let total = entries.len();
    let mut sel = app.help_sel();
    if total == 0 {
        sel = 0;
    } else if sel >= total {
        sel = total - 1;
    }

    let mut lines: Vec<Line> = Vec::new();
    let pos = if total > 0 {
        format!("  ({}/{})", sel + 1, total)
    } else {
        String::new()
    };
    let mut title = vec![
        Span::styled(
            " ? ".to_string(),
            theme.header_badge_style(theme.filter_accent()),
        ),
        Span::styled(" Help ".to_string(), theme.header_title_style()),
        Span::styled(pos, theme.header_muted_style()),
    ];
    let used: usize = title
        .iter()
        .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
        .sum();
    if used < width {
        title.push(Span::styled(" ".repeat(width - used), theme.status_style()));
    }
    lines.push(Line::from(title));

    let search = if app.help_searching() || !app.help_query().is_empty() {
        let prefix = take_width(" / ", width.saturating_sub(1)).0;
        let (query, caret) = query_viewport(
            app.help_query(),
            app.help_query().chars().count(),
            width.saturating_sub(prefix.width()),
        );
        if app.help_searching() && height >= 2 {
            frame.set_cursor_position((
                area.x + (prefix.width() + caret).min(width.saturating_sub(1)) as u16,
                area.y + 1,
            ));
        }
        format!("{prefix}{query}")
    } else {
        " / to search keys".to_string()
    };
    let pad = width.saturating_sub(UnicodeWidthStr::width(search.as_str()));
    lines.push(Line::from(Span::styled(
        format!("{search}{}", " ".repeat(pad)),
        theme.prompt_style(),
    )));
    lines.push(Line::from(Span::styled(
        "─".repeat(width),
        theme.header_muted_style(),
    )));

    let header_n = lines.len();
    let visible = height.saturating_sub(header_n + 1).max(1);
    let mut start = app.help_scroll();
    if sel < start {
        start = sel;
    }
    if sel >= start + visible {
        start = sel + 1 - visible;
    }
    let max_start = total.saturating_sub(visible);
    if start > max_start {
        start = max_start;
    }
    let end = (start + visible).min(total);

    for (row_i, entry) in entries.iter().enumerate().take(end).skip(start) {
        let selected = row_i == sel;
        let style = if selected {
            theme.match_style(true).add_modifier(Modifier::BOLD)
        } else {
            theme.prompt_style()
        };
        let key_style = if selected {
            theme.match_style(true).add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(theme.filter_accent())
                .add_modifier(Modifier::BOLD)
        };
        let prefix = if selected { "▌ " } else { "  " };
        let key = truncate(entry.key, 22);
        let key_pad = format!("{:<22}", key);
        let rest_w = width.saturating_sub(2 + 22 + 1);
        let desc = truncate(entry.desc, rest_w);
        let mut row = vec![
            Span::styled(prefix.to_string(), style),
            Span::styled(key_pad, key_style),
            Span::styled(" ".to_string(), style),
            Span::styled(desc, style),
        ];
        let used: usize = row
            .iter()
            .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
            .sum();
        if used < width {
            row.push(Span::styled(" ".repeat(width - used), style));
        }
        lines.push(Line::from(row));
    }

    while lines.len() < height.saturating_sub(1) {
        lines.push(Line::from(Span::styled(
            " ".repeat(width),
            theme.detail_style(),
        )));
    }
    lines.push(Line::from(Span::styled(
        truncate("  j/k move · / search · esc close · ? toggle ", width),
        theme.header_muted_style(),
    )));

    let body = Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height: height as u16,
    };
    frame.render_widget(Paragraph::new(lines), body);
    if let Some(reserved) = reserved_bottom_rows(area) {
        frame.render_widget(Clear, reserved);
    }
}

pub fn drawable_area(frame: Rect, geometry: Option<PaneGeometry>) -> Rect {
    let Some(geometry) = geometry else {
        return frame;
    };
    Rect {
        x: frame.x,
        y: frame.y,
        width: frame.width,
        height: frame.height.min(geometry.height),
    }
}

fn drawable_body_area_with_detail(area: Rect, detail_height: u16) -> Rect {
    Rect {
        x: area.x,
        y: if area.height >= 2 {
            area.y.saturating_add(2)
        } else {
            area.y
        },
        width: area.width,
        height: area.height.saturating_sub(
            RESERVED_BOTTOM_ROWS
                .saturating_add(2)
                .saturating_add(detail_height),
        ),
    }
}

fn detail_rows_available(area: Rect) -> u16 {
    // Reserve space for header, prompt, at least 3 list rows, and bottom chrome.
    area.height
        .saturating_sub(RESERVED_BOTTOM_ROWS.saturating_add(2).saturating_add(3))
        .min(4)
}

fn drawable_status_area(area: Rect) -> Option<Rect> {
    if area.height == 0 {
        return None;
    }
    Some(Rect {
        y: area.y,
        height: 1,
        ..area
    })
}

fn reserved_bottom_rows(area: Rect) -> Option<Rect> {
    if area.height <= RESERVED_BOTTOM_ROWS + 2 {
        return None;
    }
    Some(Rect {
        y: area.y + area.height - RESERVED_BOTTOM_ROWS,
        height: RESERVED_BOTTOM_ROWS,
        ..area
    })
}

fn draw_header(frame: &mut Frame<'_>, app: &ExtractApp, area: Rect) {
    let Some(header_area) = drawable_status_area(area) else {
        return;
    };
    let theme = app.theme();
    let width = usize::from(header_area.width);
    let mode = app.mode().badge();
    let engine = app.engine().name().to_ascii_uppercase();
    let filtered = app.filtered_count();
    let total = app.total_count();
    let marked = app.marked_count();
    let kind_filter = app.kind_filter();

    // Build a badge-first header that stays legible as width shrinks.
    let mut spans: Vec<Span> = Vec::new();
    let push = |spans: &mut Vec<Span>, text: String, style: Style| {
        spans.push(Span::styled(text, style));
    };

    push(&mut spans, " EXTRACT ".into(), theme.header_title_style());
    let field = app.field_mode().badge();
    let field_accent = match app.field_mode() {
        crate::extract_app::FieldMode::Insert => theme.path_fg,
        crate::extract_app::FieldMode::Normal => theme.command_fg,
    };
    push(
        &mut spans,
        format!(" {field} "),
        theme.header_badge_style(field_accent),
    );
    push(
        &mut spans,
        format!(" {mode} "),
        theme.header_badge_style(theme.mode_accent()),
    );
    // Search semantics take priority over the optional engine badge.
    let notice = match (app.relaxed_search(), app.status_message()) {
        (true, Some(message)) => format!("RELAXED · {}", truncate(message, 20)),
        (true, None) => "RELAXED: ANY WORD".to_string(),
        (false, Some(message)) if app.filtered_count() > 0 => truncate(message, 20),
        _ => engine,
    };
    push(
        &mut spans,
        format!(" {notice} "),
        theme.header_badge_style(theme.engine_accent()),
    );
    if let Some(kind) = kind_filter {
        push(
            &mut spans,
            format!(" {} ", kind.stable_key().to_ascii_uppercase()),
            theme.header_badge_style(theme.filter_accent()),
        );
    }
    push(
        &mut spans,
        format!(" {filtered}/{total} "),
        theme.header_muted_style(),
    );
    if marked > 0 {
        push(
            &mut spans,
            format!(" ★{marked} "),
            theme.header_badge_style(theme.mark_fg),
        );
    }

    let used: usize = spans
        .iter()
        .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
        .sum();
    let hints = if width.saturating_sub(used) >= 56 {
        "  tab scope · ? help · ^k kind · space · enter · esc "
    } else if width.saturating_sub(used) >= 36 {
        "  tab scope · ? · ^k · space · esc "
    } else if width.saturating_sub(used) >= 18 {
        "  tab scope · ? esc "
    } else {
        ""
    };
    if !hints.is_empty() {
        push(&mut spans, hints.into(), theme.header_muted_style());
    }

    // Fill remaining header width with status bg so the bar looks solid.
    let used: usize = spans
        .iter()
        .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
        .sum();
    if used < width {
        push(&mut spans, " ".repeat(width - used), theme.status_style());
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), header_area);
}

fn draw_prompt(frame: &mut Frame<'_>, app: &ExtractApp, area: Rect) {
    if area.height < 2 {
        return;
    }
    let theme = app.theme();
    let prompt_area = Rect {
        y: area.y + 1,
        height: 1,
        ..area
    };
    let width = usize::from(prompt_area.width);
    let glyph = if theme.use_icons { " 󰍉 " } else { " > " };
    let glyph = take_width(glyph, width.saturating_sub(1)).0;
    let mut spans = vec![Span::styled(glyph.clone(), theme.prompt_accent_style())];
    let query = app.query();
    let mut kind_hint = app
        .kind_filter()
        .map(|k| format!("[{}] ", k.stable_key()))
        .unwrap_or_default();
    if glyph.width() + kind_hint.width() >= width {
        kind_hint.clear();
    }
    let prefix_width = glyph.width() + kind_hint.width();
    let query_width = width.saturating_sub(prefix_width);
    let (visible_query, caret_column) = query_viewport(query, app.query_cursor(), query_width);
    if app.field_mode() == crate::extract_app::FieldMode::Insert {
        frame.set_cursor_position((
            area.x + (prefix_width + caret_column).min(width.saturating_sub(1)) as u16,
            area.y + 1,
        ));
    }
    if !kind_hint.is_empty() {
        spans.push(Span::styled(
            kind_hint.clone(),
            theme
                .prompt_style()
                .fg(theme.filter_accent())
                .add_modifier(Modifier::BOLD),
        ));
    }
    if query.is_empty() && kind_hint.is_empty() {
        // Ghost placeholder for discoverability (does not affect filtering).
        spans.push(Span::styled(
            match app.field_mode() {
                crate::extract_app::FieldMode::Insert => {
                    "type to filter · url · @mail · esc NORMAL · ? help"
                }
                crate::extract_app::FieldMode::Normal => {
                    "NORMAL · i type · j/k move · space mark · ? help"
                }
            }
            .to_string(),
            theme
                .prompt_style()
                .fg(theme.header_muted_fg)
                .add_modifier(Modifier::ITALIC),
        ));
    } else {
        spans.push(Span::styled(
            visible_query,
            theme.prompt_style().add_modifier(Modifier::BOLD),
        ));
    }
    let used: usize = spans
        .iter()
        .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
        .sum();
    if used < width {
        spans.push(Span::styled(" ".repeat(width - used), theme.prompt_style()));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), prompt_area);
}

/// Keep the logical caret visible without splitting a displayed grapheme.
fn query_viewport(query: &str, cursor: usize, width: usize) -> (String, usize) {
    if width == 0 {
        return (String::new(), 0);
    }
    let caret_byte = query
        .char_indices()
        .nth(cursor)
        .map_or(query.len(), |(i, _)| i);
    let caret = query
        .grapheme_indices(true)
        .map(|(i, _)| i)
        .chain(std::iter::once(query.len()))
        .take_while(|&i| i <= caret_byte)
        .last()
        .unwrap_or(0);
    let mut start = caret;
    for (i, _) in query[..caret].grapheme_indices(true).rev() {
        if query[i..caret].width() >= width {
            break;
        }
        start = i;
    }
    (
        take_width(&query[start..], width).0,
        query[start..caret].width(),
    )
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DetailLine {
    prefix: String,
    text: String,
}

/// Compose the selected item into display lines without dropping any of its
/// text. The first line carries the kind chip; continuation lines are aligned
/// with the item's text. The caller decides how many of the resulting lines
/// fit in the available pane height.
fn selected_detail_lines(app: &ExtractApp, width: usize) -> Vec<DetailLine> {
    let Some(item) = app.selected_item() else {
        return Vec::new();
    };
    compose_detail_lines(item, width, app.theme().use_icons)
}

fn compose_detail_lines(item: &ExtractItem, width: usize, use_icons: bool) -> Vec<DetailLine> {
    if width == 0 {
        return Vec::new();
    }

    let width = width.saturating_sub(usize::from(width > 1)); // selection rail
    let prefix = format!(
        "  {} {} ",
        kind_icon(item.kind, use_icons),
        item.kind.stable_key().to_ascii_uppercase()
    );
    // A prefix that leaves no room for even one character would itself make
    // the selected value appear clipped. On very narrow panes, drop the
    // decoration and give the complete value all available columns.
    let (first_prefix, continuation_prefix) = if prefix.width() < width {
        (prefix.clone(), " ".repeat(prefix.width()))
    } else {
        (String::new(), String::new())
    };
    let first_width = width.saturating_sub(first_prefix.width());
    let continuation_width = width.saturating_sub(continuation_prefix.width());
    let chunks = wrap_detail_text(&item.text, first_width, continuation_width);

    chunks
        .into_iter()
        .enumerate()
        .map(|(index, text)| DetailLine {
            prefix: if index == 0 {
                first_prefix.clone()
            } else {
                continuation_prefix.clone()
            },
            text,
        })
        .collect()
}

fn wrap_detail_text(text: &str, first_width: usize, continuation_width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut first_line = true;

    for logical_line in text.split('\n') {
        if logical_line.is_empty() {
            lines.push(String::new());
            first_line = false;
            continue;
        }

        let mut remaining = logical_line;
        while !remaining.is_empty() {
            let width = if first_line {
                first_width
            } else {
                continuation_width
            };
            let (chunk, consumed) = take_width(remaining, width);
            if consumed == 0 {
                // This only occurs when a caller supplies no usable width.
                // Keep the value intact rather than silently dropping it.
                lines.push(remaining.to_string());
                remaining = "";
            } else {
                lines.push(chunk);
                remaining = &remaining[consumed..];
            }
            first_line = false;
        }
    }

    // split('\n') yields one empty logical line for an empty string and for a
    // trailing newline, so this also preserves an explicitly empty selection.
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

fn take_width(text: &str, max_width: usize) -> (String, usize) {
    if max_width == 0 {
        return (String::new(), 0);
    }
    let mut used_width = 0;
    let mut end = 0;
    for (index, grapheme) in text.grapheme_indices(true) {
        let grapheme_width = grapheme.width();
        if used_width + grapheme_width > max_width {
            break;
        }
        used_width += grapheme_width;
        end = index + grapheme.len();
    }
    (text[..end].to_string(), end)
}

fn draw_detail(frame: &mut Frame<'_>, app: &ExtractApp, area: Rect, lines: &[DetailLine]) {
    let Some(item) = app.selected_item() else {
        return;
    };
    let theme = app.theme();
    let accent = theme.kind_color(item.kind);
    let bg = theme.detail_bg;
    let text_style = Style::default().fg(theme.detail_fg).bg(bg);
    let prefix_style = Style::default()
        .fg(accent)
        .bg(bg)
        .add_modifier(Modifier::BOLD);
    let rail_style = Style::default().fg(theme.header_muted_fg).bg(bg);
    let fill = Style::default().bg(bg);
    let width = usize::from(area.width);
    let rail = if width <= 1 {
        ""
    } else if theme.use_icons {
        "│"
    } else {
        "|"
    };
    let rendered = lines
        .iter()
        .take(usize::from(area.height))
        .enumerate()
        .map(|(index, line)| {
            let mut text = line.text.clone();
            if index + 1 == usize::from(area.height) && lines.len() > usize::from(area.height) {
                let available = width.saturating_sub(rail.width() + line.prefix.width());
                text = take_width(&text, available.saturating_sub(1)).0;
                if available > 0 {
                    text.push('…');
                }
            }
            let mut spans = vec![
                Span::styled(rail.to_string(), rail_style),
                Span::styled(line.prefix.clone(), prefix_style),
                Span::styled(text, text_style),
            ];
            let used: usize = spans
                .iter()
                .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
                .sum();
            if used < width {
                spans.push(Span::styled(" ".repeat(width - used), fill));
            }
            Line::from(spans)
        })
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(rendered), area);
}

fn render_body(app: &ExtractApp, max_rows: usize, width: usize) -> Vec<Line<'static>> {
    if max_rows == 0 {
        return Vec::new();
    }
    let rows = app.visible_match_items();
    if rows.is_empty() {
        let msg = app
            .message()
            .unwrap_or("no matches — try another query or ^k kind");
        let pad = if width > 4 { "  " } else { "" };
        return vec![Line::from(Span::styled(
            format!("{pad}{}", truncate(msg, width.saturating_sub(pad.len()))),
            app.theme().empty_style(),
        ))];
    }

    // Keep the selection visible by windowing around it.
    let selected = app.selected_index().min(rows.len().saturating_sub(1));
    let start = if rows.len() <= max_rows || selected < max_rows / 2 {
        0
    } else if selected + (max_rows - max_rows / 2) >= rows.len() {
        rows.len() - max_rows
    } else {
        selected - max_rows / 2
    };
    let end = (start + max_rows).min(rows.len());

    rows[start..end]
        .iter()
        .map(|(is_selected, is_marked, item, positions)| {
            render_row(
                app.theme(),
                *is_selected,
                *is_marked,
                item,
                positions,
                width,
            )
        })
        .collect()
}

fn render_row(
    theme: &crate::theme::Theme,
    selected: bool,
    marked: bool,
    item: &ExtractItem,
    positions: &[usize],
    width: usize,
) -> Line<'static> {
    if width == 0 {
        return Line::default();
    }
    // Selection rail + multi-mark star (fzf-inspired).
    let (rail, mark) = if theme.use_icons {
        match (selected, marked) {
            (true, true) => ("▌", "★"),
            (true, false) => ("▌", " "),
            (false, true) => ("│", "★"),
            (false, false) => (" ", " "),
        }
    } else {
        match (selected, marked) {
            (true, true) => (">", "*"),
            (true, false) => (">", " "),
            (false, true) => ("|", "*"),
            (false, false) => (" ", " "),
        }
    };
    let icon = kind_icon(item.kind, theme.use_icons);
    let chip = format!(" {} {} ", icon, item.kind.stable_key().to_ascii_uppercase());
    let prefix_width = UnicodeWidthStr::width(rail) + UnicodeWidthStr::width(mark) + 1;
    let chip_width = UnicodeWidthStr::width(chip.as_str());
    let available = width.saturating_sub(prefix_width + chip_width);
    let truncated = item.text.width() > available;
    let content_width = available.saturating_sub(usize::from(truncated));
    let (visible, _) = take_width(&item.text, content_width);
    let base_style = row_style(theme, item.kind, selected);
    let match_style = if selected {
        base_style.add_modifier(Modifier::UNDERLINED | Modifier::BOLD)
    } else {
        Style::default()
            .fg(theme.match_fg)
            .add_modifier(Modifier::UNDERLINED | Modifier::BOLD)
    };
    let mut spans = vec![
        Span::styled(rail.to_string(), base_style),
        Span::styled(mark.to_string(), theme.mark_style(selected)),
        Span::styled(" ".to_string(), base_style),
    ];
    spans.push(Span::styled(
        chip,
        theme.kind_chip_style(item.kind, selected),
    ));
    let mut char_index = 0;
    for grapheme in visible.graphemes(true) {
        let char_end = char_index + grapheme.chars().count();
        let style = if positions.iter().any(|p| (char_index..char_end).contains(p)) {
            match_style
        } else {
            base_style
        };
        spans.push(Span::styled(grapheme.to_string(), style));
        char_index = char_end;
    }
    if truncated && available > 0 {
        spans.push(Span::styled("…", base_style));
    }
    // Full-width selection wash (gh-dash selected row).
    let rendered_width = spans
        .iter()
        .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
        .sum::<usize>();
    let padding = width.saturating_sub(rendered_width);
    if padding > 0 {
        spans.push(Span::styled(" ".repeat(padding), base_style));
    }
    Line::from(spans)
}

fn row_style(theme: &crate::theme::Theme, kind: ItemKind, is_selected: bool) -> Style {
    if is_selected {
        theme.match_style(true).add_modifier(Modifier::BOLD)
    } else {
        theme.row_text_style(kind, false)
    }
}

/// Build the top header with the active mode, counts, and inline key hints.
pub fn status_text(
    width: usize,
    mode: crate::extract_app::ExtractMode,
    query: &str,
    filtered: usize,
    total: usize,
    message: &str,
) -> String {
    status_text_with_engine(
        width,
        mode,
        ExtractionEngine::Regex,
        query,
        filtered,
        total,
        message,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn status_text_with_engine(
    width: usize,
    mode: crate::extract_app::ExtractMode,
    engine: ExtractionEngine,
    query: &str,
    filtered: usize,
    total: usize,
    message: &str,
    kind_filter: Option<ItemKind>,
) -> String {
    let _ = (query, message);
    let mode = mode.badge();
    let engine = engine.name().to_ascii_uppercase();
    let kind_badge = kind_filter
        .map(|kind| format!(":{}", kind.stable_key().to_ascii_uppercase()))
        .unwrap_or_default();
    let variants = [
        format!(" EXTRACT  {mode}  {engine}{kind_badge}  {filtered}/{total}  · tab · ^k · space · enter · esc "),
        format!(" EXTRACT  {mode}  {engine}{kind_badge}  {filtered}/{total}  · tab · ^k · space "),
        format!(" EXTRACT {mode} {engine}{kind_badge} {filtered}/{total}"),
        format!(" EXTRACT {mode}{kind_badge} {filtered}/{total}"),
        format!(" EXTRACT {mode} {filtered}/{total}"),
        format!(" EXTRACT {mode}"),
        " EXTRACT".to_string(),
    ];
    let text = variants
        .into_iter()
        .find(|candidate| candidate.chars().count() <= width)
        .unwrap_or_else(|| " EXTRACT ".to_string());
    text.chars().take(width).collect()
}

fn kind_icon(kind: ItemKind, use_icons: bool) -> &'static str {
    if !use_icons {
        return match kind {
            ItemKind::Url => "U",
            ItemKind::Path => "P",
            ItemKind::Error => "!",
            ItemKind::Command => ">",
            ItemKind::Hash => "#",
            ItemKind::Version => "V",
            ItemKind::Quote | ItemKind::SQuote => "\"",
            ItemKind::Code => "C",
            ItemKind::Email => "@",
            ItemKind::Ip => "I",
            ItemKind::Uuid => "Q",
            ItemKind::Host => "H",
            ItemKind::Id => "#",
            ItemKind::Env => "$",
            ItemKind::Phrase => "N",
            ItemKind::Date => "D",
            ItemKind::Color => "K",
            ItemKind::Word => "·",
        };
    }
    match kind {
        ItemKind::Url => "󰖟",
        ItemKind::Path => "󰉋",
        ItemKind::Error => "󰅚",
        ItemKind::Command => "󰆍",
        ItemKind::Hash => "󰛢",
        ItemKind::Version => "󰏗",
        ItemKind::Quote | ItemKind::SQuote => "󰸥",
        ItemKind::Code => "󰌠",
        ItemKind::Email => "󰇮",
        ItemKind::Ip => "󰩟",
        ItemKind::Uuid => "󰌱",
        ItemKind::Host => "󰒋",
        ItemKind::Id => "󰓹",
        ItemKind::Env => "󰌋",
        ItemKind::Phrase => "󰓎",
        ItemKind::Date => "󰃭",
        ItemKind::Color => "󰏘",
        ItemKind::Word => "󰊕",
    }
}

fn truncate(s: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    if s.width() <= width {
        return s.to_string();
    }
    let mut out = take_width(s, width - 1).0;
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract_app::ExtractMode;
    use pretty_assertions::assert_eq;

    #[test]
    fn status_text_keeps_help_when_width_allows() {
        let text = status_text(140, ExtractMode::Scrollback, "path", 3, 12, "");
        assert!(text.contains("enter") || text.contains("⏎"));
        assert!(text.contains("esc"));
        assert!(text.contains("tab"));
        assert!(text.contains("SCROLLBACK"));
        assert!(text.chars().count() <= 140);
    }

    #[test]
    fn status_text_shows_the_active_mode() {
        let scrollback = status_text(100, ExtractMode::Scrollback, "-", 4, 9, "");
        assert!(scrollback.contains("SCROLLBACK"));
        let global = status_text(100, ExtractMode::Global, "-", 4, 9, "");
        assert!(global.contains("GLOBAL"));
    }

    #[test]
    fn status_text_fits_narrow_width() {
        let text = status_text(10, ExtractMode::Global, "abc", 1, 2, "");
        assert!(text.chars().count() <= 10);
        assert!(text.contains("GLOBAL") || text.contains("EXTRACT"));
    }

    #[test]
    fn status_text_shows_counts() {
        let text = status_text(80, ExtractMode::Scrollback, "-", 4, 9, "");
        assert!(text.contains("4/9"));
    }

    #[test]
    fn detail_budgets_rail_and_marks_capped_content() {
        for width in [63, 120] {
            let item = ExtractItem {
                text: format!("/{}TAILZ", "x".repeat(width - 15)),
                kind: ItemKind::Path,
            };
            let lines = compose_detail_lines(&item, width, false);
            assert_eq!(lines.len(), 2);
            assert_eq!(
                lines.iter().map(|l| l.text.as_str()).collect::<String>(),
                item.text
            );
            assert!(lines
                .iter()
                .all(|l| 1 + l.prefix.width() + l.text.width() <= width));
            let long = ExtractItem {
                text: format!("/{}TAILZ", "漢".repeat(1000)),
                kind: ItemKind::Path,
            };
            let app = ExtractApp::new(vec![long], crate::theme::Theme::default());
            let mut terminal =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(width as u16, 24))
                    .unwrap();
            terminal.draw(|frame| draw(frame, &app)).unwrap();
            let final_detail: String = (0..width as u16)
                .map(|x| terminal.backend().buffer()[(x, 20)].symbol())
                .collect();
            assert!(final_detail.trim_end().ends_with('…'));
        }
    }

    #[test]
    fn active_help_search_has_a_visible_scrolling_caret() {
        use crate::extract_app::ExtractInput;
        for height in [2, 4, 24] {
            let mut app = ExtractApp::new(Vec::new(), crate::theme::Theme::default());
            app.handle_input(ExtractInput::ToggleHelp);
            app.handle_input(ExtractInput::Char('/'));
            app.handle_paste(&format!("{}TAILZ", "漢".repeat(100)));
            let mut terminal =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(63, height)).unwrap();
            terminal.draw(|frame| draw(frame, &app)).unwrap();
            let cursor = terminal.get_cursor_position().unwrap();
            assert_eq!(cursor.y, 1);
            assert!(cursor.x < 63);
            let prompt: String = (0..63)
                .map(|x| terminal.backend().buffer()[(x, 1)].symbol())
                .collect();
            assert!(prompt.contains("TAILZ"), "height={height}");
        }
    }

    #[test]
    fn relaxed_results_have_a_visible_notice_and_strict_results_clear_it() {
        use crate::extract_app::ExtractInput;
        for width in [63, 120] {
            let mut app = ExtractApp::new(
                vec![ExtractItem {
                    text: "SAPPHIRE ORBIT".into(),
                    kind: ItemKind::Code,
                }],
                crate::theme::Theme::default(),
            );
            for c in "SAPPHIRE MISSING".chars() {
                app.handle_input(ExtractInput::Char(c));
            }
            let mut terminal =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, 24)).unwrap();
            terminal.draw(|frame| draw(frame, &app)).unwrap();
            let header = |terminal: &ratatui::Terminal<ratatui::backend::TestBackend>| -> String {
                (0..width)
                    .map(|x| terminal.backend().buffer()[(x, 0)].symbol())
                    .collect()
            };
            assert!(header(&terminal).contains("RELAXED: ANY WORD"));
            app.set_message(Some("mode read failed".into()));
            terminal.draw(|frame| draw(frame, &app)).unwrap();
            assert!(header(&terminal).contains("RELAXED"));
            assert!(header(&terminal).contains("mode read failed"));
            app.handle_input(ExtractInput::ClearQuery);
            for c in "SAPPHIRE ORBIT".chars() {
                app.handle_input(ExtractInput::Char(c));
            }
            terminal.draw(|frame| draw(frame, &app)).unwrap();
            assert!(!header(&terminal).contains("RELAXED"));
        }
    }

    #[test]
    fn unicode_rows_preserve_clusters_and_budget_display_cells() {
        let theme = crate::theme::Theme::default();
        for text in ["漢字".repeat(100), "e\u{301}".repeat(100), "👩‍💻".repeat(100)] {
            let item = ExtractItem {
                text,
                kind: ItemKind::Code,
            };
            let line = render_row(&theme, true, false, &item, &[1], 63);
            let rendered: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
            assert_eq!(rendered.width(), 63);
            assert!(rendered.trim_end().ends_with('…'), "{rendered:?}");
        }
        for text in ["Cafe\u{301}", "👩‍💻"] {
            let item = ExtractItem {
                text: text.into(),
                kind: ItemKind::Code,
            };
            let line = render_row(&theme, true, false, &item, &[3], 63);
            assert!(line
                .spans
                .iter()
                .any(|s| s.content.contains(if text.starts_with('C') {
                    "e\u{301}"
                } else {
                    text
                })));
        }
    }

    #[test]
    fn long_query_view_follows_edits_and_short_prompt_survives() {
        use crate::extract_app::ExtractInput;
        for height in [2, 3, 4, 5, 24] {
            let mut app = ExtractApp::new(Vec::new(), crate::theme::Theme::default());
            for c in format!("{}TAILZ", "a".repeat(100)).chars() {
                app.handle_input(ExtractInput::Char(c));
            }
            let mut terminal =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(63, height)).unwrap();
            terminal.draw(|frame| draw(frame, &app)).unwrap();
            let row = |terminal: &ratatui::Terminal<ratatui::backend::TestBackend>| -> String {
                (0..63)
                    .map(|x| terminal.backend().buffer()[(x, 1)].symbol())
                    .collect()
            };
            assert!(row(&terminal).contains("TAILZ"), "height={height}");
            app.handle_input(ExtractInput::Left);
            app.handle_input(ExtractInput::Char('Q'));
            terminal.draw(|frame| draw(frame, &app)).unwrap();
            assert!(row(&terminal).contains("TAILQZ"), "height={height}");
            let caret = terminal.get_cursor_position().unwrap();
            assert_eq!(
                terminal.backend().buffer()[(caret.x, caret.y)].symbol(),
                "Z"
            );
        }
    }

    #[test]
    fn truncate_adds_ellipsis() {
        assert_eq!(truncate("abcdef", 4), "abc…");
        assert_eq!(truncate("ab", 4), "ab");
    }

    #[test]
    fn selected_detail_wrap_preserves_the_complete_string() {
        let item = ExtractItem {
            text: "/home/jon/.pi/agent-sessions/2026-09-11/session--long-path".to_string(),
            kind: ItemKind::Path,
        };
        let lines = compose_detail_lines(&item, 24, false);
        let reconstructed: String = lines.iter().map(|line| line.text.as_str()).collect();

        assert_eq!(reconstructed, item.text);
        assert!(lines.len() > 1);
        assert!(lines.iter().all(|line| !line.text.contains('…')));
    }

    #[test]
    fn selected_detail_drops_decoration_when_the_pane_is_too_narrow() {
        let item = ExtractItem {
            text: "abcdef".to_string(),
            kind: ItemKind::Word,
        };
        let lines = compose_detail_lines(&item, 3, false);

        assert_eq!(
            lines
                .iter()
                .map(|line| line.text.as_str())
                .collect::<String>(),
            item.text
        );
        assert!(lines.iter().all(|line| line.prefix.is_empty()));
    }

    #[test]
    fn selected_detail_tracks_navigation_and_typeahead_selection() {
        let first = "/tmp/first-long-path";
        let second = "/tmp/second-long-path";
        let mut app = ExtractApp::new(
            [first, second]
                .into_iter()
                .map(|text| ExtractItem {
                    text: text.to_string(),
                    kind: ItemKind::Path,
                })
                .collect(),
            crate::theme::Theme::default(),
        );

        let displayed = |app: &ExtractApp| {
            selected_detail_lines(app, 80)
                .iter()
                .map(|line| line.text.as_str())
                .collect::<String>()
        };
        assert_eq!(displayed(&app), first);

        app.handle_input(crate::extract_app::ExtractInput::Down);
        assert_eq!(displayed(&app), second);

        app.handle_input(crate::extract_app::ExtractInput::Char('f'));
        assert_eq!(displayed(&app), first);
    }

    #[test]
    fn row_style_applies_match_colors_for_selected_and_unselected() {
        use crate::theme::Theme;
        use ratatui::style::Color;

        let theme = Theme {
            match_fg: Color::Rgb(0xfa, 0xb3, 0x87),
            match_bg: Some(Color::Rgb(0x31, 0x32, 0x44)),
            selected_match_fg: Color::Rgb(0x11, 0x11, 0x1b),
            selected_match_bg: Color::Rgb(0xfa, 0xb3, 0x87),
            ..Theme::default()
        };

        let selected = row_style(&theme, ItemKind::Word, true);
        assert_eq!(selected.fg, Some(Color::Rgb(0x11, 0x11, 0x1b)));
        assert_eq!(selected.bg, Some(Color::Rgb(0xfa, 0xb3, 0x87)));
        assert!(selected.add_modifier.contains(Modifier::BOLD));
        assert!(!selected.add_modifier.contains(Modifier::REVERSED));

        let unselected = row_style(&theme, ItemKind::Word, false);
        // Unselected words use muted header color, no dim wash.
        assert_eq!(unselected.fg, Some(theme.header_muted_fg));
        assert!(unselected.bg.is_none() || unselected.bg == theme.match_bg);
        assert!(!unselected.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn narrow_geometry_renders_the_full_frame_width_without_clipping_the_selection() {
        let long_path =
            "/long/lab/path/selected-detail-keeps-rendering-past-the-narrow-pane-rectangle-x"
                .to_string();
        let app = ExtractApp::new(
            vec![
                ExtractItem {
                    text: long_path.clone(),
                    kind: ItemKind::Path,
                },
                ExtractItem {
                    text: "https://example.com/short".to_string(),
                    kind: ItemKind::Url,
                },
            ],
            crate::theme::Theme {
                use_icons: false,
                ..crate::theme::Theme::default()
            },
        );

        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                draw_with_visible_geometry(
                    frame,
                    &app,
                    Some(PaneGeometry {
                        width: 30,
                        height: 12,
                    }),
                )
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let row_text = |y: u16| -> String {
            (0..80u16)
                .map(|x| buffer[(x, y)].symbol().to_string())
                .collect()
        };

        // Header, prompt, and body chrome still render at the full PTY width.
        assert!(row_text(0).contains("SCROLLBACK") || row_text(0).contains("EXTRACT"));
        let prompt = row_text(1);
        assert!(
            prompt.contains('>') || prompt.contains('󰍉'),
            "prompt row missing search glyph: {prompt:?}"
        );

        // Row 2 is the active match in the list, directly below the search box.
        let match_row = row_text(2);
        assert!(
            match_row.contains("PATH") && match_row.contains("/long/lab/path"),
            "match row missing path content: {match_row:?}"
        );

        // The selected detail is soft-wrapped at the bottom of the window (rows 7-8),
        // so its content fills columns beyond the 30-column pane rectangle.
        let detail_row_1 = row_text(7);
        let detail_row_2 = row_text(8);
        assert!(detail_row_1.trim_end().chars().count() > 30);
        let reconstructed = format!("{}{}", detail_row_1.trim_end(), detail_row_2.trim_end());
        assert!(
            reconstructed.contains(&long_path)
                || reconstructed.replace(' ', "").contains("long/lab/path"),
            "wrapped detail lost path: {reconstructed:?}"
        );
        assert!(!reconstructed.contains('…'));
    }

    #[test]
    fn drawable_area_keeps_full_frame_width_with_narrow_server_geometry() {
        let frame = Rect::new(2, 3, 80, 24);
        assert_eq!(
            drawable_area(
                frame,
                Some(crate::herdr_client::PaneGeometry {
                    width: 30,
                    height: 12
                })
            ),
            Rect::new(2, 3, 80, 12)
        );
    }

    #[test]
    fn drawable_area_keeps_existing_frame_when_server_area_is_larger_or_unknown() {
        let frame = Rect::new(0, 0, 80, 24);
        let geometry = Some(crate::herdr_client::PaneGeometry {
            width: 100,
            height: 40,
        });
        assert_eq!(drawable_area(frame, geometry), frame);
        assert_eq!(drawable_area(frame, None), frame);
    }

    #[test]
    fn drawable_layout_puts_header_at_top_and_reserves_measured_chrome_rows() {
        let area = Rect::new(2, 3, 30, 12);

        assert_eq!(
            drawable_body_area_with_detail(area, 0),
            Rect::new(2, 5, 30, 7)
        );
        assert_eq!(
            drawable_body_area_with_detail(area, 3),
            Rect::new(2, 5, 30, 4)
        );
        assert_eq!(drawable_status_area(area), Some(Rect::new(2, 3, 30, 1)));
        assert_eq!(reserved_bottom_rows(area), Some(Rect::new(2, 12, 30, 3)));
    }

    #[test]
    fn drawable_layout_keeps_header_visible_in_a_short_area() {
        let area = Rect::new(0, 4, 20, 1);

        assert_eq!(
            drawable_body_area_with_detail(area, 0),
            Rect::new(0, 4, 20, 0)
        );
        assert_eq!(drawable_status_area(area), Some(Rect::new(0, 4, 20, 1)));
        assert_eq!(reserved_bottom_rows(area), None);
    }
}
