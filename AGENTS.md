# Repository Guidelines

`herdr-extractor` is the standalone laktak/extrakto-inspired scrollback token picker for Herdr.
It must remain separate from the `RooseveltAdvisors.herdr-leap` jump overlay.

## Project shape

- `herdr-plugin.toml`: plugin id `RooseveltAdvisors.herdr-extractor`, action and pane both `extract`.
- `scripts/open-extractor`: action launcher with stale-`HERDR_BIN_PATH` fallback.
- `src/extract.rs`: pure scrollback token extraction and soft-wrap reconstruction.
- `src/extract_app.rs`: pure typeahead/selection state machine.
- `src/extract_ui.rs`: ratatui renderer.
- `src/herdr_client.rs`: bounded Unix-socket calls for scrollback text, layout, and notifications.
- `src/clipboard.rs`: OSC 52 copy.

Keep pure extraction and state behavior covered by unit tests. Preserve the public lineage credit to
`laktak/extrakto` in README, LICENSE notes, and manifest metadata.

## Development

```bash
cargo fmt -- --check
cargo test
cargo build --release --locked
cargo clippy --all-targets -- -D warnings
```

Never commit `target/`, runtime logs, or local editor files.

## Maintaining this file

Update this file only for durable repository-wide guidance. Prefer pointers to authoritative files
and commands over duplicated implementation details, and remove stale guidance when behavior moves.
