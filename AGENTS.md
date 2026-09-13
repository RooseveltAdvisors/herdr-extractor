# Repository Guidelines

`herdr-extractor` is the standalone laktak/extrakto-inspired scrollback token picker for Herdr.
It must remain separate from the `RooseveltAdvisors.herdr-leap` jump overlay.

## Project shape

- `herdr-plugin.toml`: plugin id `RooseveltAdvisors.herdr-extractor`; actions `extract` (scrollback,
  pane `extract`) and `extract_transcript` (full retained transcript, pane `extract-transcript`).
- `scripts/open-extractor`: action launcher taking the entrypoint id as `$1`; stale-`HERDR_BIN_PATH` fallback.
- `src/extract.rs`: pure scrollback token extraction and soft-wrap reconstruction.
- `src/nlp.rs`: optional bounded Unix-socket sidecar protocol; no model is bundled.
- `src/extract_app.rs`: pure typeahead/selection state machine plus the `ExtractMode`
  (scrollback/global) toggle state; `ctrl+g` and `Tab` inside the picker request a mode switch through
  `Outcome::SwitchMode` and `src/main.rs` re-reads the pane source live, keeping the filter query.
- `src/extract_ui.rs`: ratatui renderer (top header, query prompt, fuzzy-match highlighting).
- `src/herdr_client.rs`: bounded Unix-socket calls for scrollback/transcript text, pane layout and
  scroll state, and notifications.
- `src/clipboard.rs`: OSC 52 copy.
- The pane entrypoint (`HERDR_PLUGIN_ENTRYPOINT_ID`: `extract` vs `extract-transcript`) selects the
  initial read mode in `src/main.rs`; the transcript/global read never falls back to
  viewport-shaped sources. The in-picker `ctrl+g`/`Tab` toggle is the primary path between modes; the
  `extract_transcript` action only exists to land directly in global mode.

Keep pure extraction and state behavior covered by unit tests. Preserve the public lineage credit to
`laktak/extrakto` in README, LICENSE notes, and manifest metadata.

The extractor's stable semantic kinds are `url`, `path`, `quote`, `squote`, `word`, `command`,
`hash`, `version`, `error`, and `code`. Their colors/icons and the optional `[nlp]` sidecar are
configured in README's Configuration section; regex extraction is the default and fallback.

## Development

```bash
cargo fmt -- --check
cargo test
cargo build --release --locked
cargo clippy --all-targets -- -D warnings
```

For independent runtime proof, use `.claude/skills/verify/SKILL.md` from a committed feature
branch. Its Herdr lab must be isolated from the operator's default session and real scrollback, and
use a throwaway tmux socket. When the lab helper (`fm-herdr-lab.sh`) is mandated by the task brief,
named-session isolation replaces the scrubbed-HOME instance; drive the built binary directly with
`HERDR_SOCKET_PATH` plus a synthetic `HERDR_PLUGIN_CONTEXT_JSON` instead of registering a dev
plugin into the operator's shared plugin registry.

Herdr facts this repo relies on (verified against 0.7.3-0.8.2 sources and live 0.8.2):
`pane.read recent_unwrapped` lines are server-capped at 1000; alt-screen panes keep no host
scrollback and report `scroll.max_offset_from_bottom = 0` via `pane.get`. A live 0.9.0 lab
also proved that an overlay in a three-pane tab can receive a 60x20 PTY while
`pane.layout` reports a 30x20 pane rectangle (`zoomed: true`); the pane buffer still contains
the full rendered status row. Issue #3799 tracks this upstream geometry regression. The
extractor queries that reported rectangle at startup and on terminal resize, clamps drawing to
its height only (the full PTY width is always used), and anchors the top header to survive the
clipped bottom chrome; `RESERVED_BOTTOM_ROWS` documents the measured three-row decoration reserve. If the layout
query is unavailable it retains the normal PTY area.

README's Demo section embeds the marketing assets in `docs/demo/` (one GIF, four screenshots).
Regenerate them by driving the release binary against a stand-in Herdr API socket -- a throwaway
tmux session plus `HERDR_SOCKET_PATH` and a synthetic `HERDR_PLUGIN_CONTEXT_JSON`, recorded with
`asciinema` and rendered with `agg` -- never by starting Herdr. Keep the fixtures synthetic and
keep the captions describing what the binary actually printed.

Never commit `target/`, runtime logs, or local editor files.

## Maintaining this file

Update this file only for durable repository-wide guidance. Prefer pointers to authoritative files
and commands over duplicated implementation details, and remove stale guidance when behavior moves.
