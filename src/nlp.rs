//! Optional local NLP sidecar client.
//!
//! The sidecar is intentionally a tiny line-delimited JSON protocol. There is
//! no model or runtime dependency in this binary; every call is bounded and a
//! caller can immediately retain the regex result on any error.

#[cfg(test)]
use std::io::{BufRead, BufReader};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::json;
use socket2::{Domain, SockAddr, Socket, Type};

use crate::extract::{ItemKind, NlpCandidate};

const SIDECAR_TIMEOUT: Duration = Duration::from_millis(180);
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

#[derive(Debug, Deserialize)]
struct Response {
    version: u32,
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
    let deadline = Instant::now() + SIDECAR_TIMEOUT;
    let request = json!({ "version": 1, "text": pane_text }).to_string() + "\n";
    let socket = Socket::new(Domain::UNIX, Type::STREAM, None)?;
    socket
        .connect_timeout(&SockAddr::unix(socket_path)?, remaining(deadline)?)
        .with_context(|| format!("cannot connect to NLP sidecar at {}", socket_path.display()))?;
    let fd: std::os::fd::OwnedFd = socket.into();
    let mut stream = UnixStream::from(fd);
    let mut pending = request.as_bytes();
    while !pending.is_empty() {
        stream.set_write_timeout(Some(remaining(deadline)?))?;
        match stream.write(pending) {
            Ok(0) => anyhow::bail!("NLP sidecar closed during request"),
            Ok(n) => pending = &pending[n..],
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e).context("NLP sidecar write failed"),
        }
    }
    let mut line = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        stream.set_read_timeout(Some(remaining(deadline)?))?;
        let n = match stream.read(&mut chunk) {
            Ok(0) => anyhow::bail!("NLP sidecar closed before completing response"),
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e).context("NLP sidecar read failed"),
        };
        let newline = chunk[..n].iter().position(|&b| b == b'\n');
        let end = newline.map_or(n, |i| i + 1);
        if line.len() + end > MAX_RESPONSE_BYTES {
            anyhow::bail!("NLP sidecar response exceeds {MAX_RESPONSE_BYTES} bytes");
        }
        line.extend_from_slice(&chunk[..end]);
        if newline.is_some() {
            break;
        }
    }
    remaining(deadline)?;
    let response: Response =
        serde_json::from_slice(&line).context("NLP sidecar returned invalid JSON")?;
    if response.version != 1 {
        anyhow::bail!("unsupported NLP sidecar version {}", response.version);
    }
    Ok(response
        .candidates
        .into_iter()
        .filter_map(|candidate| {
            if !candidate.confidence.is_finite() || !(0.0..=1.0).contains(&candidate.confidence) {
                return None;
            }
            Some(NlpCandidate {
                text: candidate.text,
                kind: ItemKind::from_stable_key(&candidate.kind)?,
                confidence: candidate.confidence,
            })
        })
        .collect())
}

fn remaining(deadline: Instant) -> Result<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|d| !d.is_zero())
        .context("NLP sidecar request deadline exceeded")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn rejects_bad_versions_oversized_frames_and_progress_past_deadline() {
        for scenario in ["version2", "missing", "oversize", "drip"] {
            let path = std::env::temp_dir().join(format!(
                "herdr-nlp-{scenario}-{}.sock",
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            let listener = UnixListener::bind(&path).unwrap();
            let handle = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = String::new();
                BufReader::new(stream.try_clone().unwrap())
                    .read_line(&mut request)
                    .unwrap();
                match scenario {
                    "version2" => {
                        let _ = stream.write_all(b"{\"version\":2,\"candidates\":[]}\n");
                    }
                    "missing" => {
                        let _ = stream.write_all(b"{\"candidates\":[]}\n");
                    }
                    "oversize" => {
                        let _ = stream.write_all(&vec![b' '; MAX_RESPONSE_BYTES + 1]);
                    }
                    _ => {
                        for _ in 0..20 {
                            if stream.write_all(b" ").is_err() {
                                break;
                            }
                            std::thread::sleep(Duration::from_millis(50));
                        }
                        let _ = stream.write_all(b"{\"version\":1,\"candidates\":[]}\n");
                    }
                }
            });
            let start = Instant::now();
            let result = request_candidates(&path, "test");
            assert!(result.is_err(), "{scenario}");
            assert!(start.elapsed() < Duration::from_millis(1000), "{scenario}");
            handle.join().unwrap();
            std::fs::remove_file(path).unwrap();
        }
    }

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
