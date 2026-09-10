use std::path::Path;
use std::process::ExitCode;

use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use herdr_extractor::clipboard::copy_to_clipboard;
use herdr_extractor::config::load_extract_settings;
use herdr_extractor::extract_app::{ExtractApp, ExtractInput, ExtractMode};
use herdr_extractor::herdr_client::{
    context_focused_pane_id, PaneGeometry, PaneText, SocketClient,
};
use herdr_extractor::Outcome;

const TRANSCRIPT_ENTRYPOINT: &str = "extract-transcript";

/// True when the plugin pane was opened through the transcript entrypoint.
fn is_transcript_entrypoint(entrypoint: Option<&str>) -> bool {
    entrypoint == Some(TRANSCRIPT_ENTRYPOINT)
}

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
    let transcript_mode =
        is_transcript_entrypoint(std::env::var("HERDR_PLUGIN_ENTRYPOINT_ID").ok().as_deref());
    let initial_mode = if transcript_mode {
        ExtractMode::Global
    } else {
        ExtractMode::Scrollback
    };
    let mut client = SocketClient::connect(Path::new(&socket_path))?;
    let config_dir = std::env::var_os("HERDR_PLUGIN_CONFIG_DIR");
    let settings = load_extract_settings(config_dir.as_deref().map(Path::new))?;
    let load = load_mode(&mut client, &pane_id, initial_mode, settings.copy_toast)?;
    if load.text.truncated {
        log_state("scrollback_truncated=true");
    }
    let mut app = ExtractApp::new(
        herdr_extractor::extract::extract_items_from_visible_text_with_wrap_width(
            &load.text.text,
            load.wrap_width,
        ),
        settings.theme.clone(),
    )
    .in_mode(initial_mode);
    log_state(&format!(
        "start mode={} source={} items={} wrap_width={:?} no_retained_history={} copy_toast={}",
        initial_mode.name(),
        load.text.source.name(),
        app.total_count(),
        load.wrap_width,
        load.no_retained_history,
        settings.copy_toast
    ));

    let outcome = run_tui(&mut app, &mut client, &pane_id, settings.copy_toast)?;
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

fn run_tui(
    app: &mut ExtractApp,
    client: &mut SocketClient,
    pane_id: &str,
    copy_toast: bool,
) -> Result<Outcome> {
    let _restore = TerminalRestore;
    let mut terminal = ratatui::init();
    let mut geometry = read_pane_geometry(client, pane_id);
    loop {
        terminal.draw(|frame| {
            herdr_extractor::extract_ui::draw_with_visible_geometry(frame, app, geometry)
        })?;
        match event::read()? {
            Event::Key(key) => {
                if let Some(input) = key_to_input(key) {
                    match app.handle_input(input) {
                        Outcome::Continue => {}
                        Outcome::SwitchMode(mode) => {
                            app.set_message(Some(format!("reading {} history...", mode.name())));
                            terminal.draw(|frame| {
                                herdr_extractor::extract_ui::draw_with_visible_geometry(
                                    frame, app, geometry,
                                )
                            })?;
                            match load_mode(client, pane_id, mode, copy_toast) {
                                Ok(load) => {
                                    if load.text.truncated {
                                        log_state("scrollback_truncated=true");
                                    }
                                    let items =
                                        herdr_extractor::extract::extract_items_from_visible_text_with_wrap_width(
                                            &load.text.text,
                                            load.wrap_width,
                                        );
                                    app.apply_mode(mode, items);
                                    log_state(&format!(
                                        "mode_switch mode={} source={} items={} wrap_width={:?} no_retained_history={}",
                                        mode.name(),
                                        load.text.source.name(),
                                        app.total_count(),
                                        load.wrap_width,
                                        load.no_retained_history,
                                    ));
                                }
                                Err(error) => {
                                    log_state(&format!(
                                        "mode_switch_error mode={}: {error:#}",
                                        mode.name()
                                    ));
                                    app.set_message(Some("mode read failed".to_string()));
                                }
                            }
                        }
                        other => return Ok(other),
                    }
                }
            }
            Event::Resize(_, _) => {
                geometry = read_pane_geometry(client, pane_id);
            }
            _ => {}
        }
    }
}

fn read_pane_geometry(client: &mut SocketClient, pane_id: &str) -> Option<PaneGeometry> {
    match client.visible_pane_geometry(pane_id) {
        Ok(geometry) => Some(geometry),
        Err(error) => {
            log_state(&format!("pane_geometry_unavailable: {error:#}"));
            None
        }
    }
}

/// Everything one mode read contributes to the picker.
struct ModeLoad {
    text: PaneText,
    wrap_width: Option<usize>,
    no_retained_history: bool,
}

/// Read the pane text for a data mode.
///
/// Scrollback mode reads the retained scrollback (with the viewport-width
/// fallback unwrap). Global mode prefers the pane's full saved session
/// history and falls back to the server-capped `recent_unwrapped` transcript,
/// never degrading to viewport-shaped sources.
fn load_mode(
    client: &mut SocketClient,
    pane_id: &str,
    mode: ExtractMode,
    copy_toast: bool,
) -> Result<ModeLoad> {
    match mode {
        ExtractMode::Scrollback => {
            let text = client.read_scrollback_pane(pane_id)?;
            let wrap_width = if text.source.is_unwrapped() {
                None
            } else {
                match client.visible_pane_geometry(pane_id) {
                    Ok(geometry) => Some(visible_wrap_width(usize::from(geometry.width))),
                    Err(error) => {
                        log_state(&format!("pane_width_unavailable: {error:#}"));
                        None
                    }
                }
            };
            Ok(ModeLoad {
                text,
                wrap_width,
                no_retained_history: false,
            })
        }
        ExtractMode::Global => {
            if let Some(history) = client.read_session_history_pane(pane_id)? {
                return Ok(ModeLoad {
                    text: history,
                    wrap_width: None,
                    no_retained_history: false,
                });
            }
            let text = client.read_transcript_pane(pane_id)?;
            if text.truncated {
                log_state(&format!(
                    "transcript_note: server capped the transcript read at 1000 lines; covered {} lines; full-session coverage needs experimental.pane_history = true in the herdr config",
                    text.text.lines().count()
                ));
                if copy_toast {
                    show_notification(
                        client,
                        "herdr-extractor: transcript limited to the last 1000 lines; set experimental.pane_history = true in herdr config for full-session coverage",
                    );
                }
            }
            let no_retained_history = match client.pane_scroll(pane_id) {
                Ok(scroll) => !scroll.has_retained_history(),
                Err(error) => {
                    log_state(&format!("pane_scroll_unavailable: {error:#}"));
                    false
                }
            };
            if no_retained_history {
                log_state(
                    "transcript_note: pane retains no host scrollback (alt-screen pane?); extracted viewport content only",
                );
                if copy_toast {
                    show_notification(
                        client,
                        "herdr-extractor: pane keeps no host scrollback; viewport only",
                    );
                }
            }
            Ok(ModeLoad {
                text,
                wrap_width: None,
                no_retained_history,
            })
        }
    }
}

fn show_notification(client: &mut SocketClient, title: &str) {
    match client.show_notification(title) {
        Ok(result) if !result.shown => {
            log_state(&format!("notification_not_shown reason={}", result.reason));
        }
        Ok(_) => {}
        Err(error) => log_state(&format!("notification_error: {error:#}")),
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
        KeyCode::Tab => Some(ExtractInput::SwitchMode),
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
    fn manifest_owns_the_extractor_actions_and_panes() {
        let value: toml::Value = toml::from_str(include_str!("../herdr-plugin.toml")).unwrap();
        assert_eq!(
            value.get("id").and_then(|id| id.as_str()),
            Some("RooseveltAdvisors.herdr-extractor")
        );
        let actions = value["actions"].as_array().unwrap();
        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0]["id"].as_str(), Some("extract"));
        assert_eq!(actions[1]["id"].as_str(), Some("extract_transcript"));
        assert_eq!(
            actions[1]["command"].as_array().unwrap()[1].as_str(),
            Some("extract-transcript")
        );
        let panes = value["panes"].as_array().unwrap();
        assert_eq!(panes.len(), 2);
        assert_eq!(panes[0]["id"].as_str(), Some("extract"));
        assert_eq!(panes[1]["id"].as_str(), Some("extract-transcript"));
        assert_eq!(
            panes[1]["command"].as_array().unwrap()[0].as_str(),
            Some("./target/release/herdr-extractor")
        );
    }

    #[test]
    fn launcher_falls_back_when_herdr_bin_path_is_stale() {
        let script = include_str!("../scripts/open-extractor");
        assert!(script.contains("[ -x \"$HERDR_BIN_PATH\" ]"));
        assert!(script.contains("command -v herdr"));
        assert!(script.contains("RooseveltAdvisors.herdr-extractor"));
    }

    #[test]
    fn launcher_defaults_to_extract_and_accepts_the_transcript_entrypoint() {
        let script = include_str!("../scripts/open-extractor");
        assert!(script.contains("entrypoint=${1:-extract}"));
        assert!(script.contains("extract|extract-transcript) ;;"));
    }

    #[test]
    fn transcript_mode_follows_the_transcript_entrypoint() {
        assert!(is_transcript_entrypoint(Some("extract-transcript")));
        assert!(!is_transcript_entrypoint(Some("extract")));
        assert!(!is_transcript_entrypoint(None));
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
            key_to_input(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
            Some(ExtractInput::SwitchMode)
        );
        assert_eq!(
            key_to_input(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::CONTROL)),
            None
        );
        assert_eq!(
            key_to_input(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE)),
            Some(ExtractInput::Char('x'))
        );
    }

    #[test]
    fn transcript_entrypoint_opens_in_global_mode() {
        let initial_mode = |entrypoint: Option<&str>| {
            if is_transcript_entrypoint(entrypoint) {
                ExtractMode::Global
            } else {
                ExtractMode::Scrollback
            }
        };
        assert_eq!(
            initial_mode(Some("extract-transcript")),
            ExtractMode::Global
        );
        assert_eq!(initial_mode(Some("extract")), ExtractMode::Scrollback);
        assert_eq!(initial_mode(None), ExtractMode::Scrollback);
    }

    #[test]
    fn visible_width_excludes_terminal_right_edge() {
        assert_eq!(visible_wrap_width(80), 79);
        assert_eq!(visible_wrap_width(1), 1);
    }
}
