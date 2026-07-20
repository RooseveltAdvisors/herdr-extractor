use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

const RPC_IO_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug)]
pub struct SocketClient {
    socket_path: PathBuf,
    next_id: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationResult {
    pub shown: bool,
    pub reason: String,
}

impl SocketClient {
    pub fn connect(socket_path: &Path) -> Result<Self> {
        UnixStream::connect(socket_path).with_context(|| {
            format!(
                "cannot connect to Herdr API socket at {}",
                socket_path.display()
            )
        })?;
        Ok(Self {
            socket_path: socket_path.to_path_buf(),
            next_id: 1,
        })
    }

    pub fn read_visible_pane(&mut self, pane_id: &str) -> Result<String> {
        let result = self.call(
            "pane.read",
            json!({
                "pane_id": pane_id,
                "source": "visible",
                "format": "text",
                "strip_ansi": true
            }),
        )?;
        expect_type(&result, "pane_read")?;
        Ok(result["read"]["text"]
            .as_str()
            .unwrap_or_default()
            .to_string())
    }

    pub fn visible_pane_width(&mut self, pane_id: &str) -> Result<usize> {
        let result = self.call("pane.layout", json!({ "pane_id": pane_id }))?;
        expect_type(&result, "pane_layout")?;
        let panes = result["layout"]["panes"]
            .as_array()
            .context("pane_layout result did not include panes")?;
        let pane = panes
            .iter()
            .find(|pane| pane["pane_id"].as_str() == Some(pane_id))
            .context("pane_layout result did not include the requested pane")?;
        let width = pane["rect"]["width"]
            .as_u64()
            .context("pane_layout result did not include the pane width")?;
        usize::try_from(width).context("pane width did not fit in usize")
    }

    pub fn show_notification(&mut self, title: &str) -> Result<NotificationResult> {
        let result = self.call("notification.show", json!({ "title": title }))?;
        expect_type(&result, "notification_show")?;
        Ok(NotificationResult {
            shown: result["shown"].as_bool().unwrap_or(false),
            reason: result["reason"].as_str().unwrap_or("unknown").to_string(),
        })
    }

    fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id.to_string();
        self.next_id += 1;
        let mut stream = UnixStream::connect(&self.socket_path)
            .with_context(|| format!("cannot connect to Herdr API while calling {method}"))?;
        stream
            .set_read_timeout(Some(RPC_IO_TIMEOUT))
            .with_context(|| format!("cannot set Herdr API read deadline for {method}"))?;
        stream
            .set_write_timeout(Some(RPC_IO_TIMEOUT))
            .with_context(|| format!("cannot set Herdr API write deadline for {method}"))?;

        let mut request = json!({"id": id, "method": method, "params": params}).to_string();
        request.push('\n');
        stream
            .write_all(request.as_bytes())
            .with_context(|| format!("Herdr API failed while writing request for {method}"))?;

        let mut response = String::new();
        if BufReader::new(stream).read_line(&mut response)? == 0 {
            bail!("Herdr closed the API connection before answering {method}");
        }
        let envelope: Value = serde_json::from_str(&response)
            .with_context(|| format!("Herdr returned invalid JSON for {method}"))?;
        if let Some(error) = envelope.get("error") {
            bail!(
                "Herdr API error {}: {}",
                error["code"].as_str().unwrap_or("unknown_error"),
                error["message"].as_str().unwrap_or("no message")
            );
        }
        if envelope["id"].as_str() != Some(&id) {
            bail!("Herdr response id did not match request id {id}");
        }
        envelope
            .get("result")
            .cloned()
            .context("Herdr response has neither result nor error")
    }
}

fn expect_type(result: &Value, expected: &str) -> Result<()> {
    let actual = result["type"].as_str().unwrap_or("<missing>");
    if actual != expected {
        bail!("expected {expected} result, got {actual}");
    }
    Ok(())
}

pub fn context_focused_pane_id() -> Option<String> {
    let context: Value =
        serde_json::from_str(&std::env::var("HERDR_PLUGIN_CONTEXT_JSON").ok()?).ok()?;
    context
        .get("focused_pane_id")?
        .as_str()
        .map(ToString::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn socket_path() -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        PathBuf::from(format!("/tmp/herdr-extractor-{unique}.sock"))
    }

    #[test]
    fn reads_only_visible_text_and_layout_width() {
        let path = socket_path();
        let listener = UnixListener::bind(&path).unwrap();
        let handle = std::thread::spawn(move || {
            let _probe = listener.accept().unwrap();
            let (mut stream, _) = listener.accept().unwrap();
            let mut request_line = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut request_line)
                .unwrap();
            let request: Value = serde_json::from_str(&request_line).unwrap();
            assert_eq!(request["method"], "pane.read");
            assert_eq!(request["params"]["source"], "visible");
            stream
                .write_all(b"{\"id\":\"1\",\"result\":{\"type\":\"pane_read\",\"read\":{\"text\":\"visible\"}}}\n")
                .unwrap();

            let (mut stream, _) = listener.accept().unwrap();
            request_line.clear();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut request_line)
                .unwrap();
            let request: Value = serde_json::from_str(&request_line).unwrap();
            assert_eq!(request["method"], "pane.layout");
            stream
                .write_all(b"{\"id\":\"2\",\"result\":{\"type\":\"pane_layout\",\"layout\":{\"panes\":[{\"pane_id\":\"w1:p1\",\"rect\":{\"width\":80}}]}}}\n")
                .unwrap();
        });

        let mut client = SocketClient::connect(&path).unwrap();
        assert_eq!(client.read_visible_pane("w1:p1").unwrap(), "visible");
        assert_eq!(client.visible_pane_width("w1:p1").unwrap(), 80);
        handle.join().unwrap();
        let _ = std::fs::remove_file(path);
    }
}
