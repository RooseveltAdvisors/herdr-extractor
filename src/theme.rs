use anyhow::{bail, Result};
use ratatui::style::{Color, Modifier, Style};

use crate::extract::ItemKind;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Theme {
    pub match_fg: Color,
    pub match_bg: Option<Color>,
    pub selected_match_fg: Color,
    pub selected_match_bg: Color,
    pub status_fg: Color,
    pub status_bg: Color,
    pub empty_fg: Color,
    pub url_fg: Color,
    pub path_fg: Color,
    pub error_fg: Color,
    pub command_fg: Color,
    pub hash_fg: Color,
    pub version_fg: Color,
    pub quote_fg: Color,
    pub code_fg: Color,
    pub use_icons: bool,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            match_fg: Color::Gray,
            match_bg: None,
            selected_match_fg: Color::Rgb(0, 0, 0),
            selected_match_bg: Color::Rgb(223, 142, 29),
            status_fg: Color::Black,
            status_bg: Color::Gray,
            empty_fg: Color::Gray,
            url_fg: Color::Gray,
            path_fg: Color::Cyan,
            error_fg: Color::Gray,
            command_fg: Color::Gray,
            hash_fg: Color::Gray,
            version_fg: Color::Gray,
            quote_fg: Color::DarkGray,
            code_fg: Color::Gray,
            use_icons: detect_icons(),
        }
    }
}

impl Theme {
    pub fn match_style(&self, selected: bool) -> Style {
        if selected {
            Style::default()
                .fg(self.selected_match_fg)
                .bg(self.selected_match_bg)
        } else {
            style_with_optional_bg(self.match_fg, self.match_bg)
        }
    }

    pub fn status_style(&self) -> Style {
        Style::default().fg(self.status_fg).bg(self.status_bg)
    }

    pub fn empty_style(&self) -> Style {
        Style::default().fg(self.empty_fg)
    }

    pub fn kind_style(&self, kind: ItemKind, selected: bool) -> Style {
        let foreground = match kind {
            ItemKind::Url => self.url_fg,
            ItemKind::Path => self.path_fg,
            ItemKind::Error => self.error_fg,
            ItemKind::Command => self.command_fg,
            ItemKind::Hash => self.hash_fg,
            ItemKind::Version => self.version_fg,
            ItemKind::Quote | ItemKind::SQuote => self.quote_fg,
            ItemKind::Code => self.code_fg,
            ItemKind::Word => self.match_fg,
        };
        if selected {
            self.match_style(true)
        } else {
            let mut style = style_with_optional_bg(foreground, self.match_bg);
            if matches!(kind, ItemKind::Quote | ItemKind::SQuote) {
                style = style.add_modifier(Modifier::ITALIC | Modifier::DIM);
            }
            style
        }
    }

    pub fn kind_chip_style(&self, kind: ItemKind, selected: bool) -> Style {
        self.kind_style(kind, selected)
    }
}

fn detect_icons() -> bool {
    std::env::var("HERDR_EXTRACTOR_ASCII_ICONS")
        .map(|value| value != "1" && value != "true")
        .unwrap_or_else(|_| {
            std::env::var("TERM")
                .map(|term| term != "dumb")
                .unwrap_or(true)
        })
}

fn style_with_optional_bg(fg: Color, bg: Option<Color>) -> Style {
    let style = Style::default().fg(fg);
    if let Some(bg) = bg {
        style.bg(bg)
    } else {
        style
    }
}

pub fn parse_color(input: &str) -> Result<Color> {
    let normalized = input.trim().to_ascii_lowercase();
    let color = match normalized.as_str() {
        "black" => Color::Black,
        "red" => Color::Red,
        "green" => Color::Green,
        "yellow" => Color::Yellow,
        "blue" => Color::Blue,
        "magenta" => Color::Magenta,
        "cyan" => Color::Cyan,
        "gray" | "grey" => Color::Gray,
        "dark-gray" | "dark-grey" => Color::DarkGray,
        "light-red" => Color::LightRed,
        "light-green" => Color::LightGreen,
        "light-yellow" => Color::LightYellow,
        "light-blue" => Color::LightBlue,
        "light-magenta" => Color::LightMagenta,
        "light-cyan" => Color::LightCyan,
        "white" => Color::White,
        _ if normalized.starts_with('#') => parse_hex_color(&normalized)?,
        _ => bail!("unknown color '{input}'"),
    };
    Ok(color)
}

fn parse_hex_color(input: &str) -> Result<Color> {
    let hex = input.trim_start_matches('#');
    if hex.len() != 6 {
        bail!("hex colors must use #RRGGBB, got '{input}'");
    }
    let red = u8::from_str_radix(&hex[0..2], 16)?;
    let green = u8::from_str_radix(&hex[2..4], 16)?;
    let blue = u8::from_str_radix(&hex[4..6], 16)?;
    Ok(Color::Rgb(red, green, blue))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_theme_uses_a_restrained_professional_palette() {
        let theme = Theme::default();

        assert_eq!(theme.match_fg, Color::Gray);
        assert_eq!(theme.match_bg, None);
        assert_eq!(theme.selected_match_fg, Color::Rgb(0, 0, 0));
        assert_eq!(theme.selected_match_bg, Color::Rgb(223, 142, 29));
        assert_eq!(theme.path_fg, Color::Cyan);
        assert_eq!(theme.url_fg, Color::Gray);
        assert_eq!(theme.hash_fg, Color::Gray);
        assert_eq!(theme.version_fg, Color::Gray);
        assert_ne!(theme.selected_match_fg, Color::White);
        assert_ne!(theme.selected_match_bg, Color::Magenta);
    }

    #[test]
    fn parses_named_and_hex_colors() {
        assert_eq!(parse_color("light-blue").unwrap(), Color::LightBlue);
        assert_eq!(parse_color("#112233").unwrap(), Color::Rgb(17, 34, 51));
    }

    #[test]
    fn rejects_unknown_colors() {
        assert!(parse_color("not-a-color").is_err());
    }
}
