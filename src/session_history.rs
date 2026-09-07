//! Saved session transcript source (`session-history.json`).
//!
//! Herdr's `pane.read` API caps every read at 1000 logical lines server-side, so
//! a long pane's older transcript lines are unreachable through the socket. When
//! Herdr runs with `experimental.pane_history = true`, the server persists each
//! pane's full retained (unwrapped) ANSI history next to the session state at
//! `<session data dir>/session-history.json`. This module maps the plugin's
//! focused pane id to that saved history and converts it to plain text.
//!
//! Pure file/JSON logic: no socket calls, so it stays unit-testable.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde_json::Value;

/// Herdr's public-id alphabet (`src/workspace.rs`), replicated for decoding
/// public pane numbers like the `pB` in `w1:p2`.
const PUBLIC_ID_ALPHABET: &[u8; 32] = b"123456789ABCDEFGHJKMNPQRSTVWXYZ0";

/// Load the pane's full saved session transcript as plain text.
///
/// `data_dir` is the Herdr session data directory (the parent of the API
/// socket). Returns `Ok(None)` when there is no usable saved history for the
/// pane (files missing, pane id unmappable, pane absent, or empty history);
/// the caller should then fall back to the capped `pane.read` transcript.
pub fn load_pane_history(data_dir: &Path, pane_id: &str) -> Result<Option<String>> {
    let session_path = data_dir.join("session.json");
    let history_path = data_dir.join("session-history.json");
    if !session_path.is_file() || !history_path.is_file() {
        return Ok(None);
    }
    let session: Value = serde_json::from_str(&read_file(&session_path)?)
        .with_context(|| format!("invalid JSON in {}", session_path.display()))?;
    let history: Value = serde_json::from_str(&read_file(&history_path)?)
        .with_context(|| format!("invalid JSON in {}", history_path.display()))?;
    let Some(raw_pane_id) = raw_pane_id(pane_id, &session) else {
        return Ok(None);
    };
    let Some(ansi) = pane_history_ansi(&history, raw_pane_id) else {
        return Ok(None);
    };
    Ok(Some(strip_ansi(ansi)))
}

fn read_file(path: &Path) -> Result<String> {
    fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))
}

/// Resolve the plugin pane id to the globally unique raw pane id that keys the
/// saved history map.
///
/// Handles the public form Herdr sends plugins (`w1:p2`: workspace id plus the
/// per-workspace public pane number) by looking the number up in
/// `session.json`'s `public_pane_numbers`, and the raw `p_42` / `p_w1_42`
/// alias forms directly.
fn raw_pane_id(pane_id: &str, session: &Value) -> Option<u64> {
    if let Some(rest) = pane_id.strip_prefix("p_") {
        let raw = match rest.rsplit_once('_') {
            Some((_, pane_raw)) => pane_raw,
            None => rest,
        };
        return raw.parse::<u64>().ok();
    }
    let (workspace_id, pane_number_raw) = pane_id.rsplit_once(":p")?;
    let pane_number = decode_public_number(pane_number_raw)?;
    let workspace = session["workspaces"]
        .as_array()?
        .iter()
        .find(|workspace| workspace["id"].as_str() == Some(workspace_id))?;
    let pane_numbers = workspace["public_pane_numbers"].as_object()?;
    for (raw, number) in pane_numbers {
        if number.as_u64() == Some(pane_number) {
            return raw.parse::<u64>().ok();
        }
    }
    None
}

fn decode_public_number(value: &str) -> Option<u64> {
    let mut decoded: u64 = 0;
    for character in value.chars() {
        let digit = PUBLIC_ID_ALPHABET
            .iter()
            .position(|candidate| *candidate as char == character)? as u64;
        decoded = decoded
            .checked_mul(PUBLIC_ID_ALPHABET.len() as u64)?
            .checked_add(digit + 1)?;
    }
    Some(decoded)
}

/// Find the pane's saved ANSI history. Raw pane ids are globally unique across
/// workspaces and tabs, so scan every saved pane map for the key.
fn pane_history_ansi(history: &Value, raw_pane_id: u64) -> Option<&str> {
    history["workspaces"]
        .as_array()?
        .iter()
        .filter_map(|workspace| workspace["tabs"].as_array())
        .flatten()
        .filter_map(|tab| tab["panes"].as_object())
        .find_map(|panes| panes.get(&raw_pane_id.to_string()))
        .and_then(|pane| pane["ansi"].as_str())
        .filter(|ansi| !ansi.trim().is_empty())
}

/// Convert saved ANSI history to plain text: drop escape sequences and control
/// characters while keeping printable text and line breaks.
pub fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut characters = text.chars().peekable();
    while let Some(character) = characters.next() {
        match character {
            '\x1b' => match characters.next() {
                Some('[') => {
                    // CSI: parameter/intermediate bytes, then one final byte.
                    for consumed in characters.by_ref() {
                        if ('\u{40}'..='\u{7e}').contains(&consumed) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    // OSC: terminated by BEL or ST (ESC \).
                    loop {
                        match characters.next() {
                            Some('\x07') | None => break,
                            Some('\x1b') => {
                                if characters.peek() == Some(&'\\') {
                                    characters.next();
                                }
                                break;
                            }
                            Some(_) => {}
                        }
                    }
                }
                // Two- and three-character escape forms (charset, line size).
                Some('(') | Some(')') | Some('#') => {
                    characters.next();
                }
                Some(_) | None => {}
            },
            '\r' | '\x07' | '\x08' => {}
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::extract::extract_items_from_visible_text;

    fn temp_dir(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("herdr-extractor-{name}-{unique}"));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn write_session(data_dir: &Path, pane_numbers: &str) {
        fs::write(
            data_dir.join("session.json"),
            format!(
                r#"{{"version":3,"workspaces":[{{"id":"w1","public_pane_numbers":{pane_numbers}}}]}}"#
            ),
        )
        .unwrap();
    }

    fn write_history(data_dir: &Path, panes: &str) {
        fs::write(
            data_dir.join("session-history.json"),
            format!(r#"{{"version":3,"workspaces":[{{"tabs":[{{"panes":{panes}}}]}}]}}"#),
        )
        .unwrap();
    }

    fn ansi_lines(count: usize, first_line: &str) -> String {
        let mut ansi = format!("\x1b[34m{first_line}\x1b[0m\r\n");
        for index in 1..count {
            ansi.push_str(&format!("filler line {index} plain\r\n"));
        }
        ansi
    }

    #[test]
    fn full_saved_history_exceeds_the_server_line_cap() {
        let data_dir = temp_dir("full-history");
        write_session(&data_dir, r#"{"7":1}"#);
        let history_lines = 1500;
        write_history(
            &data_dir,
            &format!(
                r#"{{"7":{{"ansi":{},"lines":{history_lines}}}}}"#,
                serde_json::to_string(&ansi_lines(
                    history_lines,
                    "early https://first.example/link in scrollback"
                ))
                .unwrap()
            ),
        );

        let text = load_pane_history(&data_dir, "w1:p1")
            .unwrap()
            .expect("saved history should cover the pane");

        assert_eq!(text.lines().count(), history_lines);
        assert!(history_lines > 1000, "fixture must exceed the server cap");
        let items: Vec<_> = extract_items_from_visible_text(&text)
            .into_iter()
            .map(|item| item.text)
            .collect();
        assert!(
            items
                .iter()
                .any(|item| item == "https://first.example/link"),
            "links near the start must survive extraction: {items:?}"
        );
        let _ = fs::remove_dir_all(&data_dir);
    }

    #[test]
    fn maps_public_pane_id_through_session_json() {
        let data_dir = temp_dir("public-id");
        write_session(&data_dir, r#"{"7":2}"#);
        write_history(
            &data_dir,
            &format!(
                r#"{{"7":{{"ansi":{},"lines":1}}}}"#,
                serde_json::to_string("saved \u{1b}[1mtoken\u{1b}[0m").unwrap()
            ),
        );
        // Sanity-check the alphabet decode herdr uses for public numbers.
        assert_eq!(decode_public_number("1"), Some(1));
        assert_eq!(decode_public_number("A"), Some(10));

        let text = load_pane_history(&data_dir, "w1:p2").unwrap().unwrap();
        assert_eq!(text, "saved token");
        let _ = fs::remove_dir_all(&data_dir);
    }

    #[test]
    fn maps_raw_p_prefixed_pane_ids() {
        let data_dir = temp_dir("raw-id");
        write_session(&data_dir, r#"{"7":1}"#);
        write_history(&data_dir, r#"{"7":{"ansi":"raw form","lines":1}}"#);

        assert_eq!(
            load_pane_history(&data_dir, "p_w1_7").unwrap().as_deref(),
            Some("raw form")
        );
        assert_eq!(
            load_pane_history(&data_dir, "p_7").unwrap().as_deref(),
            Some("raw form")
        );
        let _ = fs::remove_dir_all(&data_dir);
    }

    #[test]
    fn missing_files_or_pane_return_none() {
        let data_dir = temp_dir("missing");

        assert_eq!(load_pane_history(&data_dir, "w1:p1").unwrap(), None);

        write_session(&data_dir, r#"{"7":1}"#);
        assert_eq!(load_pane_history(&data_dir, "w1:p1").unwrap(), None);

        write_history(&data_dir, r#"{"8":{"ansi":"other pane","lines":1}}"#);
        assert_eq!(load_pane_history(&data_dir, "w1:p1").unwrap(), None);

        // Unknown pane number encoding and unknown workspaces also fall back.
        assert_eq!(load_pane_history(&data_dir, "w2:pA").unwrap(), None);
        assert_eq!(load_pane_history(&data_dir, "w1:p!").unwrap(), None);
        let _ = fs::remove_dir_all(&data_dir);
    }

    #[test]
    fn empty_saved_history_returns_none() {
        let data_dir = temp_dir("empty-history");
        write_session(&data_dir, r#"{"7":1}"#);
        write_history(&data_dir, r#"{"7":{"ansi":"  \r\n","lines":0}}"#);
        assert_eq!(load_pane_history(&data_dir, "w1:p1").unwrap(), None);
        let _ = fs::remove_dir_all(&data_dir);
    }

    #[test]
    fn strip_ansi_keeps_text_and_newlines() {
        let stripped = strip_ansi(
            "\x1b[34mblue\x1b[0m\r\n\x1b]8;;https://x.example\x07link\x1b]8;;\x07\x1b(Bmore\x1b=",
        );
        assert_eq!(stripped, "blue\nlinkmore");
    }
}
