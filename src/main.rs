use std::path::Path;
use std::process::ExitCode;

use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use herdr_extractor::clipboard::copy_to_clipboard;
use herdr_extractor::config::load_extract_settings;
use herdr_extractor::extract_app::{ExtractApp, ExtractInput};
use herdr_extractor::herdr_client::{context_focused_pane_id, SocketClient};
use herdr_extractor::Outcome;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            log_state(&format!("error: {error:#}"));
            eprintln!("herdr-extractor: {error:#}");
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<()> {
    let socket_path = std::env::var_os("HERDR_SOCKET_PATH")
        .context("HERDR_SOCKET_PATH is not set; open this through the Herdr plugin action")?;
    let pane_id = context_focused_pane_id()
        .context("HERDR_PLUGIN_CONTEXT_JSON did not include focused_pane_id")?;
    let mut client = SocketClient::connect(Path::new(&socket_path))?;
    let text = client.read_visible_pane(&pane_id)?;
    let wrap_width = match client.visible_pane_width(&pane_id) {
        Ok(width) => Some(visible_wrap_width(width)),
        Err(error) => {
            log_state(&format!("pane_width_unavailable: {error:#}"));
            None
        }
    };
    let config_dir = std::env::var_os("HERDR_PLUGIN_CONFIG_DIR");
    let settings = load_extract_settings(config_dir.as_deref().map(Path::new))?;
    let mut app =
        ExtractApp::from_visible_text_with_wrap_width(&text, wrap_width, settings.theme.clone());
    log_state(&format!(
        "start items={} wrap_width={wrap_width:?} copy_toast={}",
        app.total_count(),
        settings.copy_toast
    ));

    let outcome = run_tui(&mut app)?;
    log_state(&format!("outcome={outcome:?}"));
    if let Outcome::Copy(text) = outcome {
        copy_to_clipboard(&text)?;
        if settings.copy_toast {
            match client.show_notification(&copy_notification_title(&text)) {
                Ok(result) if !result.shown => {
                    log_state(&format!("notification_not_shown reason={}", result.reason));
                }
                Ok(_) => {}
                Err(error) => log_state(&format!("notification_error: {error:#}")),
            }
        }
    }
    Ok(())
}

fn run_tui(app: &mut ExtractApp) -> Result<Outcome> {
    let _restore = TerminalRestore;
    let mut terminal = ratatui::init();
    loop {
        terminal.draw(|frame| herdr_extractor::extract_ui::draw(frame, app))?;
        match event::read()? {
            Event::Key(key) => {
                if let Some(input) = key_to_input(key) {
                    match app.handle_input(input) {
                        Outcome::Continue => {}
                        other => return Ok(other),
                    }
                }
            }
            Event::Resize(_, _) => {}
            _ => {}
        }
    }
}

fn key_to_input(key: KeyEvent) -> Option<ExtractInput> {
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        return match key.code {
            KeyCode::Char('c') | KeyCode::Char('C') => Some(ExtractInput::CtrlC),
            KeyCode::Char('n') | KeyCode::Char('N') => Some(ExtractInput::Down),
            KeyCode::Char('p') | KeyCode::Char('P') => Some(ExtractInput::Up),
            _ => None,
        };
    }
    match key.code {
        KeyCode::Esc => Some(ExtractInput::Esc),
        KeyCode::Backspace => Some(ExtractInput::Backspace),
        KeyCode::Enter => Some(ExtractInput::Enter),
        KeyCode::Up => Some(ExtractInput::Up),
        KeyCode::Down => Some(ExtractInput::Down),
        KeyCode::Char(character) => Some(ExtractInput::Char(character)),
        _ => None,
    }
}

fn copy_notification_title(text: &str) -> String {
    let mut characters = text.chars();
    let mut preview = characters.by_ref().take(15).collect::<String>();
    if characters.next().is_some() {
        preview.push_str("...");
    }
    format!("Copied: {preview}")
}

fn visible_wrap_width(layout_width: usize) -> usize {
    if layout_width > 1 {
        layout_width - 1
    } else {
        layout_width
    }
}

fn log_state(message: &str) {
    let Some(directory) = std::env::var_os("HERDR_PLUGIN_STATE_DIR") else {
        return;
    };
    let path = Path::new(&directory).join("herdr-extractor.log");
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut file| {
            std::io::Write::write_all(&mut file, format!("{message}\n").as_bytes())
        });
}

struct TerminalRestore;

impl Drop for TerminalRestore {
    fn drop(&mut self) {
        ratatui::restore();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn manifest_owns_only_the_extractor_action_and_pane() {
        let value: toml::Value = toml::from_str(include_str!("../herdr-plugin.toml")).unwrap();
        assert_eq!(
            value.get("id").and_then(|id| id.as_str()),
            Some("RooseveltAdvisors.herdr-extractor")
        );
        let actions = value["actions"].as_array().unwrap();
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0]["id"].as_str(), Some("extract"));
        let panes = value["panes"].as_array().unwrap();
        assert_eq!(panes.len(), 1);
        assert_eq!(panes[0]["id"].as_str(), Some("extract"));
    }

    #[test]
    fn launcher_falls_back_when_herdr_bin_path_is_stale() {
        let script = include_str!("../scripts/open-extractor");
        assert!(script.contains("[ -x \"$HERDR_BIN_PATH\" ]"));
        assert!(script.contains("command -v herdr"));
        assert!(script.contains("RooseveltAdvisors.herdr-extractor"));
    }

    #[test]
    fn key_map_supports_typeahead_and_navigation() {
        assert_eq!(
            key_to_input(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            Some(ExtractInput::Enter)
        );
        assert_eq!(
            key_to_input(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL)),
            Some(ExtractInput::Down)
        );
        assert_eq!(
            key_to_input(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE)),
            Some(ExtractInput::Char('x'))
        );
    }

    #[test]
    fn visible_width_excludes_terminal_right_edge() {
        assert_eq!(visible_wrap_width(80), 79);
        assert_eq!(visible_wrap_width(1), 1);
    }
}
