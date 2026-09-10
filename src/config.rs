//! Loads optional extractor settings from `$HERDR_PLUGIN_CONFIG_DIR/config.toml`.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::theme::{parse_color, Theme};

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ExtractSettings {
    pub copy_toast: bool,
    pub theme: Theme,
    pub nlp: NlpSettings,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NlpSettings {
    pub enabled: bool,
    pub socket_path: Option<PathBuf>,
    pub confidence_threshold: f32,
}

impl Default for NlpSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            socket_path: None,
            confidence_threshold: 0.75,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RawConfig {
    #[serde(default)]
    copy_toast: bool,
    style: Option<StyleConfig>,
    nlp: Option<NlpConfig>,
    icons: Option<bool>,
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
    url_fg: Option<String>,
    path_fg: Option<String>,
    error_fg: Option<String>,
    command_fg: Option<String>,
    hash_fg: Option<String>,
    version_fg: Option<String>,
    quote_fg: Option<String>,
    code_fg: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct NlpConfig {
    #[serde(default)]
    enabled: bool,
    socket_path: Option<String>,
    confidence_threshold: Option<f32>,
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
        for (value, target, name) in [
            (&style.url_fg, &mut theme.url_fg, "style.url_fg"),
            (&style.path_fg, &mut theme.path_fg, "style.path_fg"),
            (&style.error_fg, &mut theme.error_fg, "style.error_fg"),
            (&style.command_fg, &mut theme.command_fg, "style.command_fg"),
            (&style.hash_fg, &mut theme.hash_fg, "style.hash_fg"),
            (&style.version_fg, &mut theme.version_fg, "style.version_fg"),
            (&style.quote_fg, &mut theme.quote_fg, "style.quote_fg"),
            (&style.code_fg, &mut theme.code_fg, "style.code_fg"),
        ] {
            if let Some(value) = value {
                *target = parse_color(value).with_context(|| format!("invalid {name}"))?;
            }
        }
    }
    if let Some(icons) = raw.icons {
        theme.use_icons = icons;
    }
    let nlp = raw
        .nlp
        .as_ref()
        .map_or_else(NlpSettings::default, |nlp| NlpSettings {
            enabled: nlp.enabled,
            socket_path: nlp.socket_path.clone().map(PathBuf::from),
            confidence_threshold: nlp.confidence_threshold.unwrap_or(0.75).clamp(0.0, 1.0),
        });
    Ok(ExtractSettings {
        copy_toast: raw.copy_toast,
        theme,
        nlp,
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

    #[test]
    fn nlp_is_off_by_default_and_parses_socket_settings() {
        assert!(!ExtractSettings::default().nlp.enabled);
        let raw = toml::from_str(
            "icons = false\n[nlp]\nenabled = true\nsocket_path = '/tmp/extract-nlp.sock'\nconfidence_threshold = 0.8\n",
        )
        .unwrap();
        let settings = compile_settings(&raw).unwrap();
        assert!(settings.nlp.enabled);
        assert_eq!(
            settings.nlp.socket_path,
            Some(PathBuf::from("/tmp/extract-nlp.sock"))
        );
        assert_eq!(settings.nlp.confidence_threshold, 0.8);
        assert!(!settings.theme.use_icons);
    }
}
