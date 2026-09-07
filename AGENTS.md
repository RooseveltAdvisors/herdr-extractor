# Repository Guidelines

`herdr-extractor` is the standalone laktak/extrakto-inspired scrollback token picker for Herdr.
It must remain separate from the `RooseveltAdvisors.herdr-leap` jump overlay.

## Project shape

- `herdr-plugin.toml`: plugin id `RooseveltAdvisors.herdr-extractor`; actions `extract` (scrollback,
  pane `extract`) and `extract_transcript` (full retained transcript, pane `extract-transcript`).
- `scripts/open-extractor`: action launcher taking the entrypoint id as `$1`; stale-`HERDR_BIN_PATH` fallback.
- `src/extract.rs`: pure scrollback token extraction and soft-wrap reconstruction.
- `src/extract_app.rs`: pure typeahead/selection state machine.
- `src/extract_ui.rs`: ratatui renderer.
- `src/herdr_client.rs`: bounded Unix-socket calls for scrollback/transcript text, pane layout and
  scroll state, and notifications.
- `src/clipboard.rs`: OSC 52 copy.
- The pane entrypoint (`HERDR_PLUGIN_ENTRYPOINT_ID`: `extract` vs `extract-transcript`) selects the
  read mode in `src/main.rs`; the transcript read never falls back to viewport-shaped sources.

Keep pure extraction and state behavior covered by unit tests. Preserve the public lineage credit to
`laktak/extrakto` in README, LICENSE notes, and manifest metadata.

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
scrollback and report `scroll.max_offset_from_bottom = 0` via `pane.get`.

Never commit `target/`, runtime logs, or local editor files.

## Maintaining this file

Update this file only for durable repository-wide guidance. Prefer pointers to authoritative files
and commands over duplicated implementation details, and remove stale guidance when behavior moves.
