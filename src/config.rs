//! Loads optional extractor settings from `$HERDR_PLUGIN_CONFIG_DIR/config.toml`.

use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::theme::{parse_color, Theme};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractSettings {
    pub copy_toast: bool,
    pub theme: Theme,
}

impl Default for ExtractSettings {
    fn default() -> Self {
        Self {
            copy_toast: false,
            theme: Theme::default(),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RawConfig {
    #[serde(default)]
    copy_toast: bool,
    style: Option<StyleConfig>,
}

#[derive(Debug, Clone, Deserialize)]
struct StyleConfig {
    match_fg: Option<String>,
    match_bg: Option<String>,
    selected_match_fg: Option<String>,
    selected_match_bg: Option<String>,
    status_fg: Option<String>,
    status_bg: Option<String>,
    empty_fg: Option<String>,
}

fn compile_settings(raw: &RawConfig) -> Result<ExtractSettings> {
    let mut theme = Theme::default();
    if let Some(style) = raw.style.as_ref() {
        if let Some(value) = &style.match_fg {
            theme.match_fg = parse_color(value).context("invalid style.match_fg")?;
        }
        if let Some(value) = &style.match_bg {
            theme.match_bg = Some(parse_color(value).context("invalid style.match_bg")?);
        }
        if let Some(value) = &style.selected_match_fg {
            theme.selected_match_fg =
                parse_color(value).context("invalid style.selected_match_fg")?;
        }
        if let Some(value) = &style.selected_match_bg {
            theme.selected_match_bg =
                parse_color(value).context("invalid style.selected_match_bg")?;
        }
        if let Some(value) = &style.status_fg {
            theme.status_fg = parse_color(value).context("invalid style.status_fg")?;
        }
        if let Some(value) = &style.status_bg {
            theme.status_bg = parse_color(value).context("invalid style.status_bg")?;
        }
        if let Some(value) = &style.empty_fg {
            theme.empty_fg = parse_color(value).context("invalid style.empty_fg")?;
        }
    }
    Ok(ExtractSettings {
        copy_toast: raw.copy_toast,
        theme,
    })
}

pub fn load_extract_settings(config_dir: Option<&Path>) -> Result<ExtractSettings> {
    let Some(config_dir) = config_dir else {
        return Ok(ExtractSettings::default());
    };
    let path = config_dir.join("config.toml");
    let input = match std::fs::read_to_string(&path) {
        Ok(input) => input,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ExtractSettings::default());
        }
        Err(error) => {
            return Err(error).with_context(|| format!("failed to read {}", path.display()));
        }
    };
    let raw: RawConfig =
        toml::from_str(&input).with_context(|| format!("failed to parse {}", path.display()))?;
    compile_settings(&raw)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color;

    #[test]
    fn empty_config_yields_defaults() {
        assert_eq!(
            compile_settings(&toml::from_str("").unwrap()).unwrap(),
            ExtractSettings::default()
        );
    }

    #[test]
    fn parses_copy_toast_and_list_colors() {
        let raw = toml::from_str(
            r##"
copy_toast = true
[style]
selected_match_bg = "#112233"
status_bg = "blue"
"##,
        )
        .unwrap();
        let settings = compile_settings(&raw).unwrap();
        assert!(settings.copy_toast);
        assert_eq!(settings.theme.selected_match_bg, Color::Rgb(17, 34, 51));
        assert_eq!(settings.theme.status_bg, Color::Blue);
    }

    #[test]
    fn rejects_unknown_color() {
        let raw = toml::from_str("[style]\nmatch_fg = \"wrong\"").unwrap();
        assert!(compile_settings(&raw)
            .unwrap_err()
            .to_string()
            .contains("style.match_fg"));
    }
}
