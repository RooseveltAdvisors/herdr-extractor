//! Ratatui rendering for the extract typeahead list.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use ratatui::Frame;

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

/// Draw within the pane rectangle Herdr reports, when it is narrower or
/// shorter than the picker PTY. A fixed upstream layout naturally passes the
/// full frame through unchanged.
pub fn draw_with_visible_geometry(
    frame: &mut Frame<'_>,
    app: &ExtractApp,
    geometry: Option<PaneGeometry>,
) {
    let area = drawable_area(frame.area(), geometry);
    if area.height == 0 || area.width == 0 {
        return;
    }
    // Herdr's overlay box is client-width even when pane.layout reports a
    // narrower pane rectangle. Keep the PR-8 geometry clamp for list content,
    // but use the visible frame width for captain-facing header and prompt.
    let header_area = frame.area();
    draw_header(frame, app, header_area);
    draw_prompt(frame, app, header_area);
    let body_area = drawable_body_area(area);
    let lines = render_body(app, usize::from(body_area.height), usize::from(area.width));
    frame.render_widget(Paragraph::new(lines), body_area);
    if let Some(reserved_area) = reserved_bottom_rows(area) {
        frame.render_widget(Clear, reserved_area);
    }
}

pub fn drawable_area(frame: Rect, geometry: Option<PaneGeometry>) -> Rect {
    let Some(geometry) = geometry else {
        return frame;
    };
    Rect {
        x: frame.x,
        y: frame.y,
        width: frame.width.min(geometry.width),
        height: frame.height.min(geometry.height),
    }
}

fn drawable_body_area(area: Rect) -> Rect {
    Rect {
        x: area.x,
        y: if area.height >= 2 {
            area.y.saturating_add(2)
        } else {
            area.y
        },
        width: area.width,
        height: area
            .height
            .saturating_sub(RESERVED_BOTTOM_ROWS.saturating_add(2)),
    }
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
    if area.height <= RESERVED_BOTTOM_ROWS {
        return None;
    }
    Some(Rect {
        y: area.y + area.height - RESERVED_BOTTOM_ROWS,
        height: RESERVED_BOTTOM_ROWS,
        ..area
    })
}

fn draw_header(frame: &mut Frame<'_>, app: &ExtractApp, area: Rect) {
    if let Some(header_area) = drawable_status_area(area) {
        let text = status_text_with_engine(
            usize::from(header_area.width),
            app.mode(),
            app.engine(),
            app.query(),
            app.filtered_count(),
            app.total_count(),
            app.message().unwrap_or(""),
        );
        let badge_width = text.find("  ").unwrap_or(text.len()).min(text.len());
        let badge = text[..badge_width].to_string();
        let rest = text[badge_width..].to_string();
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(badge, app.theme().kind_style(ItemKind::Command, true)),
                Span::styled(rest, app.theme().status_style().add_modifier(Modifier::DIM)),
            ])),
            header_area,
        );
    }
}

fn draw_prompt(frame: &mut Frame<'_>, app: &ExtractApp, area: Rect) {
    if area.height < 2 {
        return;
    }
    let prompt_area = Rect {
        y: area.y + 1,
        height: 1,
        ..area
    };
    let mut spans = vec![Span::styled("> ", app.theme().status_style())];
    spans.push(Span::styled(
        app.query(),
        app.theme().match_style(true).add_modifier(Modifier::BOLD),
    ));
    if let Some(item) = app.selected_item() {
        spans.push(Span::styled(
            format!("  {}", item.text),
            Style::default().add_modifier(Modifier::DIM),
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), prompt_area);
}

fn render_body(app: &ExtractApp, max_rows: usize, width: usize) -> Vec<Line<'static>> {
    if max_rows == 0 {
        return Vec::new();
    }
    let rows = app.visible_match_items();
    if rows.is_empty() {
        let msg = app.message().unwrap_or("no matches");
        return vec![Line::from(Span::styled(
            truncate(msg, width),
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
        .map(|(is_selected, item, positions)| {
            render_row(app.theme(), *is_selected, item, positions, width)
        })
        .collect()
}

fn render_row(
    theme: &crate::theme::Theme,
    selected: bool,
    item: &ExtractItem,
    positions: &[usize],
    width: usize,
) -> Line<'static> {
    if width == 0 {
        return Line::default();
    }
    let prefix = if selected { "▌ " } else { "  " };
    let icon = kind_icon(item.kind, theme.use_icons);
    let chip = format!("{} {} ", icon, item.kind.stable_key().to_ascii_uppercase());
    let available = width.saturating_sub(prefix.chars().count() + chip.chars().count());
    let text_chars: Vec<char> = item.text.chars().collect();
    let truncated = text_chars.len() > available;
    let content_len = if truncated && available > 0 {
        available.saturating_sub(1)
    } else {
        available
    };
    let base_style = row_style(theme, item.kind, selected);
    let match_style = base_style.add_modifier(Modifier::UNDERLINED);
    let mut spans = vec![Span::styled(prefix, base_style)];
    spans.push(Span::styled(
        chip,
        theme.kind_chip_style(item.kind, selected),
    ));
    for (index, character) in text_chars.iter().take(content_len).enumerate() {
        let style = if positions.contains(&index) {
            match_style
        } else {
            base_style
        };
        spans.push(Span::styled(character.to_string(), style));
    }
    if truncated && available > 0 {
        spans.push(Span::styled("…", base_style));
    }
    Line::from(spans)
}

fn row_style(theme: &crate::theme::Theme, kind: ItemKind, is_selected: bool) -> Style {
    let style = theme.kind_style(kind, is_selected);
    if is_selected {
        style.add_modifier(Modifier::BOLD | Modifier::REVERSED)
    } else {
        style.add_modifier(Modifier::DIM)
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
    )
}

pub fn status_text_with_engine(
    width: usize,
    mode: crate::extract_app::ExtractMode,
    engine: ExtractionEngine,
    query: &str,
    filtered: usize,
    total: usize,
    message: &str,
) -> String {
    let _ = (query, message);
    let mode = mode.name().to_ascii_uppercase();
    let engine = engine.name().to_ascii_uppercase();
    let variants = [
        format!(" {mode}:{engine}  {filtered}/{total}  ·  tab mode  ·  enter copy  ·  esc cancel "),
        format!(" {mode}:{engine}  {filtered}/{total}  · tab mode · enter copy · esc cancel "),
        format!(" {mode}:{engine} {filtered}/{total} · tab mode · enter copy "),
        format!(" {mode}:{engine} {filtered}/{total} · tab mode"),
        format!(" {mode}:{engine} {filtered}/{total}"),
        format!(" {mode}:{engine}"),
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
        ItemKind::Word => "󰊕",
    }
}

fn truncate(s: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let count = s.chars().count();
    if count <= width {
        return s.to_string();
    }
    if width <= 1 {
        return s.chars().take(width).collect();
    }
    let mut out: String = s.chars().take(width - 1).collect();
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
        assert!(text.contains("enter copy"));
        assert!(text.contains("esc cancel"));
        assert!(text.contains("tab mode"));
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
    fn truncate_adds_ellipsis() {
        assert_eq!(truncate("abcdef", 4), "abc…");
        assert_eq!(truncate("ab", 4), "ab");
    }

    #[test]
    fn row_style_applies_match_colors_for_selected_and_unselected() {
        use crate::theme::Theme;
        use ratatui::style::Color;

        let theme = Theme {
            match_fg: Color::Cyan,
            match_bg: Some(Color::DarkGray),
            selected_match_fg: Color::White,
            selected_match_bg: Color::Blue,
            ..Theme::default()
        };

        let selected = row_style(&theme, ItemKind::Word, true);
        assert_eq!(selected.fg, Some(Color::White));
        assert_eq!(selected.bg, Some(Color::Blue));
        assert!(selected.add_modifier.contains(Modifier::BOLD));

        let unselected = row_style(&theme, ItemKind::Word, false);
        assert_eq!(unselected.fg, Some(Color::Cyan));
        assert_eq!(unselected.bg, Some(Color::DarkGray));
        assert!(unselected.add_modifier.contains(Modifier::DIM));
        assert!(!unselected.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn drawable_area_clamps_each_axis_to_server_geometry() {
        let frame = Rect::new(2, 3, 80, 24);
        assert_eq!(
            drawable_area(
                frame,
                Some(crate::herdr_client::PaneGeometry {
                    width: 30,
                    height: 12
                })
            ),
            Rect::new(2, 3, 30, 12)
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

        assert_eq!(drawable_body_area(area), Rect::new(2, 5, 30, 7));
        assert_eq!(drawable_status_area(area), Some(Rect::new(2, 3, 30, 1)));
        assert_eq!(reserved_bottom_rows(area), Some(Rect::new(2, 12, 30, 3)));
    }

    #[test]
    fn drawable_layout_keeps_header_visible_in_a_short_area() {
        let area = Rect::new(0, 4, 20, 1);

        assert_eq!(drawable_body_area(area), Rect::new(0, 4, 20, 0));
        assert_eq!(drawable_status_area(area), Some(Rect::new(0, 4, 20, 1)));
        assert_eq!(reserved_bottom_rows(area), None);
    }
}
