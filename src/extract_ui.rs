//! Ratatui rendering for the extract typeahead list.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::extract_app::ExtractApp;

pub fn draw(frame: &mut Frame<'_>, app: &ExtractApp) {
    let area = frame.area();
    if area.height == 0 || area.width == 0 {
        return;
    }
    let body_height = area.height.saturating_sub(1);
    let body_area = Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height: body_height,
    };
    let lines = render_body(app, usize::from(body_height), usize::from(area.width));
    frame.render_widget(Paragraph::new(lines), body_area);
    draw_status(frame, app, area);
}

fn render_body(app: &ExtractApp, max_rows: usize, width: usize) -> Vec<Line<'static>> {
    if max_rows == 0 {
        return Vec::new();
    }
    let rows = app.visible_rows();
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
        .map(|(is_selected, text)| {
            let prefix = if *is_selected { "> " } else { "  " };
            let line = format!("{prefix}{text}");
            Line::from(Span::styled(
                truncate(&line, width),
                row_style(app.theme(), *is_selected),
            ))
        })
        .collect()
}

fn row_style(theme: &crate::theme::Theme, is_selected: bool) -> Style {
    let style = theme.match_style(is_selected);
    if is_selected {
        style.add_modifier(Modifier::BOLD)
    } else {
        style.add_modifier(Modifier::DIM)
    }
}

fn draw_status(frame: &mut Frame<'_>, app: &ExtractApp, area: Rect) {
    if area.height == 0 {
        return;
    }
    let status_area = Rect {
        x: area.x,
        y: area.y + area.height - 1,
        width: area.width,
        height: 1,
    };
    let text = status_text(
        usize::from(status_area.width),
        app.query(),
        app.filtered_count(),
        app.total_count(),
        app.message().unwrap_or(""),
    );
    frame.render_widget(
        Paragraph::new(text).style(app.theme().status_style()),
        status_area,
    );
}

/// Build a status line that fits within `width`.
pub fn status_text(
    width: usize,
    query: &str,
    filtered: usize,
    total: usize,
    message: &str,
) -> String {
    let q = if query.is_empty() { "-" } else { query };
    let variants = [
        format!(" extract  query:{q}  {filtered}/{total}  {message}  enter:copy  esc:cancel "),
        format!(" extract  query:{q}  {filtered}/{total}  {message}"),
        format!(" extract q:{q} {filtered}/{total}"),
        " extract".to_string(),
    ];
    let text = variants
        .into_iter()
        .find(|candidate| candidate.chars().count() <= width)
        .unwrap_or_else(|| " extract ".to_string());
    text.chars().take(width).collect()
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
    use pretty_assertions::assert_eq;

    #[test]
    fn status_text_keeps_help_when_width_allows() {
        let text = status_text(100, "path", 3, 12, "");
        assert!(text.contains("enter:copy"));
        assert!(text.contains("esc:cancel"));
        assert!(text.contains("extract"));
        assert!(text.chars().count() <= 100);
    }

    #[test]
    fn status_text_fits_narrow_width() {
        let text = status_text(10, "abc", 1, 2, "");
        assert!(text.chars().count() <= 10);
        assert!(text.contains("extract") || text.contains("ext"));
    }

    #[test]
    fn status_text_shows_counts() {
        let text = status_text(80, "-", 4, 9, "");
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

        let selected = row_style(&theme, true);
        assert_eq!(selected.fg, Some(Color::White));
        assert_eq!(selected.bg, Some(Color::Blue));
        assert!(selected.add_modifier.contains(Modifier::BOLD));

        let unselected = row_style(&theme, false);
        assert_eq!(unselected.fg, Some(Color::Cyan));
        assert_eq!(unselected.bg, Some(Color::DarkGray));
        assert!(unselected.add_modifier.contains(Modifier::DIM));
        assert!(!unselected.add_modifier.contains(Modifier::BOLD));
    }
}
