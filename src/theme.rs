//! Visual theme for the extract picker.
//!
//! Default palette is a Catppuccin Mocha-inspired dark scheme tuned for dense
//! terminal overlays: warm peach selection, saturated kind chips, quiet chrome.

use anyhow::{bail, Result};
use ratatui::style::{Color, Modifier, Style};

use crate::extract::ItemKind;

/// RGB helpers kept as named constants so the palette reads as a design system.
#[allow(dead_code)]
mod palette {
    use ratatui::style::Color;

    pub const BASE: Color = Color::Rgb(0x1e, 0x1e, 0x2e);
    pub const MANTLE: Color = Color::Rgb(0x18, 0x18, 0x25);
    pub const SURFACE0: Color = Color::Rgb(0x31, 0x32, 0x44);
    pub const SURFACE1: Color = Color::Rgb(0x45, 0x47, 0x5a);
    pub const OVERLAY0: Color = Color::Rgb(0x6c, 0x70, 0x86);
    pub const OVERLAY1: Color = Color::Rgb(0x7f, 0x84, 0x9c);
    pub const TEXT: Color = Color::Rgb(0xcd, 0xd6, 0xf4);
    pub const SUBTEXT: Color = Color::Rgb(0xa6, 0xad, 0xc8);
    pub const PEACH: Color = Color::Rgb(0xfa, 0xb3, 0x87);
    pub const YELLOW: Color = Color::Rgb(0xf9, 0xe2, 0xaf);
    pub const GREEN: Color = Color::Rgb(0xa6, 0xe3, 0xa1);
    pub const TEAL: Color = Color::Rgb(0x94, 0xe2, 0xd5);
    pub const SKY: Color = Color::Rgb(0x89, 0xdc, 0xeb);
    pub const SAPPHIRE: Color = Color::Rgb(0x74, 0xc7, 0xec);
    pub const BLUE: Color = Color::Rgb(0x89, 0xb4, 0xfa);
    pub const LAVENDER: Color = Color::Rgb(0xb4, 0xbe, 0xfe);
    pub const MAUVE: Color = Color::Rgb(0xcb, 0xa6, 0xf7);
    pub const PINK: Color = Color::Rgb(0xf5, 0xc2, 0xe7);
    pub const RED: Color = Color::Rgb(0xf3, 0x8b, 0xa8);
    pub const MAROON: Color = Color::Rgb(0xeb, 0xa0, 0xac);
    pub const BLACK: Color = Color::Rgb(0x11, 0x11, 0x1b);
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Theme {
    pub match_fg: Color,
    pub match_bg: Option<Color>,
    pub selected_match_fg: Color,
    pub selected_match_bg: Color,
    pub status_fg: Color,
    pub status_bg: Color,
    pub empty_fg: Color,
    /// Prompt chrome (search glyph + field background).
    pub prompt_fg: Color,
    pub prompt_bg: Color,
    pub prompt_accent: Color,
    /// Soft surface behind the selected-detail preview strip.
    pub detail_bg: Color,
    pub detail_fg: Color,
    pub header_title_fg: Color,
    pub header_muted_fg: Color,
    pub mark_fg: Color,
    pub url_fg: Color,
    pub path_fg: Color,
    pub error_fg: Color,
    pub command_fg: Color,
    pub hash_fg: Color,
    pub version_fg: Color,
    pub quote_fg: Color,
    pub code_fg: Color,
    pub email_fg: Color,
    pub ip_fg: Color,
    pub uuid_fg: Color,
    pub host_fg: Color,
    pub id_fg: Color,
    pub env_fg: Color,
    pub phrase_fg: Color,
    pub date_fg: Color,
    pub color_fg: Color,
    pub use_icons: bool,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            match_fg: palette::PEACH,
            match_bg: None,
            selected_match_fg: palette::BLACK,
            selected_match_bg: palette::PEACH,
            status_fg: palette::TEXT,
            status_bg: palette::SURFACE0,
            empty_fg: palette::OVERLAY1,
            prompt_fg: palette::TEXT,
            prompt_bg: palette::SURFACE0,
            prompt_accent: palette::BLUE,
            detail_bg: palette::MANTLE,
            detail_fg: palette::SUBTEXT,
            header_title_fg: palette::PEACH,
            header_muted_fg: palette::OVERLAY1,
            mark_fg: palette::YELLOW,
            url_fg: palette::BLUE,
            path_fg: palette::GREEN,
            error_fg: palette::RED,
            command_fg: palette::PEACH,
            hash_fg: palette::TEAL,
            version_fg: palette::YELLOW,
            quote_fg: palette::OVERLAY1,
            code_fg: palette::TEAL,
            email_fg: palette::MAUVE,
            ip_fg: palette::SAPPHIRE,
            uuid_fg: palette::LAVENDER,
            host_fg: palette::SKY,
            id_fg: palette::YELLOW,
            env_fg: palette::GREEN,
            phrase_fg: palette::PINK,
            date_fg: palette::SAPPHIRE,
            color_fg: palette::MAUVE,
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

    pub fn header_title_style(&self) -> Style {
        Style::default()
            .fg(self.header_title_fg)
            .bg(self.status_bg)
            .add_modifier(Modifier::BOLD)
    }

    pub fn header_muted_style(&self) -> Style {
        Style::default().fg(self.header_muted_fg).bg(self.status_bg)
    }

    pub fn header_badge_style(&self, accent: Color) -> Style {
        Style::default()
            .fg(palette::BLACK)
            .bg(accent)
            .add_modifier(Modifier::BOLD)
    }

    pub fn prompt_style(&self) -> Style {
        Style::default().fg(self.prompt_fg).bg(self.prompt_bg)
    }

    pub fn prompt_accent_style(&self) -> Style {
        Style::default()
            .fg(self.prompt_accent)
            .bg(self.prompt_bg)
            .add_modifier(Modifier::BOLD)
    }

    pub fn detail_style(&self) -> Style {
        Style::default().fg(self.detail_fg).bg(self.detail_bg)
    }

    pub fn empty_style(&self) -> Style {
        Style::default()
            .fg(self.empty_fg)
            .add_modifier(Modifier::ITALIC)
    }

    pub fn mark_style(&self, selected: bool) -> Style {
        if selected {
            self.match_style(true).add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(self.mark_fg)
                .add_modifier(Modifier::BOLD)
        }
    }

    pub fn kind_color(&self, kind: ItemKind) -> Color {
        match kind {
            ItemKind::Url => self.url_fg,
            ItemKind::Path => self.path_fg,
            ItemKind::Error => self.error_fg,
            ItemKind::Command => self.command_fg,
            ItemKind::Hash => self.hash_fg,
            ItemKind::Version => self.version_fg,
            ItemKind::Quote | ItemKind::SQuote => self.quote_fg,
            ItemKind::Code => self.code_fg,
            ItemKind::Email => self.email_fg,
            ItemKind::Ip => self.ip_fg,
            ItemKind::Uuid => self.uuid_fg,
            ItemKind::Host => self.host_fg,
            ItemKind::Id => self.id_fg,
            ItemKind::Env => self.env_fg,
            ItemKind::Phrase => self.phrase_fg,
            ItemKind::Date => self.date_fg,
            ItemKind::Color => self.color_fg,
            ItemKind::Word => self.header_muted_fg,
        }
    }

    pub fn kind_style(&self, kind: ItemKind, selected: bool) -> Style {
        if selected {
            return self.match_style(true);
        }
        let mut style = Style::default().fg(self.kind_color(kind));
        if matches!(kind, ItemKind::Quote | ItemKind::SQuote) {
            style = style.add_modifier(Modifier::ITALIC);
        }
        style
    }

    /// Inverted pill for the kind chip: kind color background, dark label.
    pub fn kind_chip_style(&self, kind: ItemKind, selected: bool) -> Style {
        if selected {
            Style::default()
                .fg(self.selected_match_fg)
                .bg(self.selected_match_bg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(palette::BLACK)
                .bg(self.kind_color(kind))
                .add_modifier(Modifier::BOLD)
        }
    }

    pub fn row_text_style(&self, kind: ItemKind, selected: bool) -> Style {
        if selected {
            self.match_style(true)
        } else {
            Style::default().fg(self.kind_color(kind))
        }
    }

    pub fn mode_accent(&self) -> Color {
        palette::BLUE
    }

    pub fn engine_accent(&self) -> Color {
        palette::TEAL
    }

    pub fn filter_accent(&self) -> Color {
        palette::MAUVE
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
    let hex = input.strip_prefix('#').unwrap_or(input);
    if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
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

        assert_eq!(theme.selected_match_fg, Color::Rgb(0x11, 0x11, 0x1b));
        assert_eq!(theme.selected_match_bg, Color::Rgb(0xfa, 0xb3, 0x87));
        assert_eq!(theme.path_fg, Color::Rgb(0xa6, 0xe3, 0xa1));
        assert_eq!(theme.url_fg, Color::Rgb(0x89, 0xb4, 0xfa));
        assert_eq!(theme.status_bg, Color::Rgb(0x31, 0x32, 0x44));
        assert_ne!(theme.selected_match_bg, Color::Magenta);
        assert_ne!(theme.match_fg, Color::Gray);
    }

    #[test]
    fn kind_chips_are_inverted_pills() {
        let theme = Theme::default();
        let chip = theme.kind_chip_style(ItemKind::Url, false);
        assert_eq!(chip.bg, Some(theme.url_fg));
        assert_eq!(chip.fg, Some(Color::Rgb(0x11, 0x11, 0x1b)));
    }

    #[test]
    fn parses_named_and_hex_colors() {
        assert_eq!(parse_color("light-blue").unwrap(), Color::LightBlue);
        assert_eq!(parse_color("#112233").unwrap(), Color::Rgb(17, 34, 51));
    }

    #[test]
    fn rejects_unknown_colors() {
        assert!(parse_color("not-a-color").is_err());
        for input in ["#€abc", "#💻ab", "#éabcd", "##aabbcc", "#aabbzz"] {
            assert!(parse_color(input).is_err(), "{input}");
        }
    }
}
