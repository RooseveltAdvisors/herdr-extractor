//! Optional local NLP sidecar client.
//!
//! The sidecar is intentionally a tiny line-delimited JSON protocol. There is
//! no model or runtime dependency in this binary; every call is bounded and a
//! caller can immediately retain the regex result on any error.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::json;

use crate::extract::{ItemKind, NlpCandidate};

const SIDECAR_TIMEOUT: Duration = Duration::from_millis(180);

#[derive(Debug, Deserialize)]
struct Response {
    #[serde(default)]
    candidates: Vec<Candidate>,
}

#[derive(Debug, Deserialize)]
struct Candidate {
    text: String,
    kind: String,
    confidence: f32,
}

/// Ask a local sidecar once. The caller owns fallback behavior and should log
/// errors rather than showing them as fatal picker failures.
pub fn request_candidates(socket_path: &Path, pane_text: &str) -> Result<Vec<NlpCandidate>> {
    let mut stream = UnixStream::connect(socket_path)
        .with_context(|| format!("cannot connect to NLP sidecar at {}", socket_path.display()))?;
    stream.set_read_timeout(Some(SIDECAR_TIMEOUT))?;
    stream.set_write_timeout(Some(SIDECAR_TIMEOUT))?;
    let request = json!({ "version": 1, "text": pane_text }).to_string() + "\n";
    stream
        .write_all(request.as_bytes())
        .context("NLP sidecar write failed")?;
    let mut line = String::new();
    if BufReader::new(stream).read_line(&mut line)? == 0 {
        anyhow::bail!("NLP sidecar closed before responding");
    }
    let response: Response =
        serde_json::from_str(&line).context("NLP sidecar returned invalid JSON")?;
    Ok(response
        .candidates
        .into_iter()
        .filter_map(|candidate| {
            Some(NlpCandidate {
                text: candidate.text,
                kind: ItemKind::from_stable_key(&candidate.kind)?,
                confidence: candidate.confidence,
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn speaks_versioned_json_lines_protocol() {
        let path = std::env::temp_dir().join(format!(
            "herdr-extractor-nlp-{}.sock",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let listener = UnixListener::bind(&path).unwrap();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut line)
                .unwrap();
            let request: serde_json::Value = serde_json::from_str(&line).unwrap();
            assert_eq!(request["version"], 1);
            assert_eq!(request["text"], "hello world");
            stream.write_all(br#"{"version":1,"candidates":[{"text":"hello world","kind":"code","confidence":0.91}]}"#).unwrap();
            stream.write_all(b"\n").unwrap();
        });
        let candidates = request_candidates(&path, "hello world").unwrap();
        assert_eq!(candidates[0].kind, ItemKind::Code);
        handle.join().unwrap();
        let _ = std::fs::remove_file(path);
    }
}
