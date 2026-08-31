# herdr-extractor

A [Herdr](https://herdr.dev) plugin that lists copy-eligible tokens from the focused pane's
retained **scrollback** and filters them with typeahead.

## Lineage and credit

This workflow follows [laktak/extrakto](https://github.com/laktak/extrakto), “quickly select,
copy/insert/complete text without a mouse.” This Herdr port is not presented as an original UX
invention. It adapts extrakto's token-picking lineage to Herdr's scrollback and OSC 52 APIs.

## Action

`RooseveltAdvisors.herdr-extractor.extract` opens the `extract` overlay entrypoint.

1. The plugin calls `pane.read` with `source = "recent_unwrapped"` and requests the maximum line
   bound when that parameter is supported, so text found while reading copy mode remains available
   after returning to normal mode. Older Herdr versions fall back to `recent`, then `visible`, only
   when a source is unsupported.
2. Herdr supplies logical lines for `recent_unwrapped`; fallback sources use the pane layout width.
3. A bounded extrakto-parity set collects URLs, paths, double/single quotes, and words of at least
   five characters. Lower/recent results come first and duplicates are removed.
4. Type to filter. `Up`/`Down` or `Ctrl-p`/`Ctrl-n` moves selection. `Enter` copies exactly one item
   through OSC 52. `Esc` or `Ctrl-C` cancels.

## Install

```bash
herdr plugin install RooseveltAdvisors/herdr-extractor
herdr server reload-config
```

```toml
[[keys.command]]
key = "prefix+space"
type = "plugin_action"
command = "RooseveltAdvisors.herdr-extractor.extract"
description = "extract a scrollback token"
```

Copy mode is for READING; `prefix+space` is for TAKING. Scroll through pane output with copy mode,
exit to normal mode, then invoke `RooseveltAdvisors.herdr-extractor.extract` with `prefix+space`.
The picker searches retained scrollback and copies the chosen result through OSC 52, so it reaches
the outer terminal clipboard (including the captain's Mac).

This action moved out of `RooseveltAdvisors.herdr-leap` in the public plugin split. Do not bind
`prefix+space` to `RooseveltAdvisors.herdr-leap.open`; that opens the separate jump workflow.

The launcher validates `HERDR_BIN_PATH` and falls back to `herdr` on `PATH`, covering a replaced
Linux server executable whose stale path ends in ` (deleted)`.

## Configuration

Create `config.toml` under `herdr plugin config-dir RooseveltAdvisors.herdr-extractor`:

```toml
copy_toast = true

[style]
selected_match_bg = "magenta"
status_bg = "gray"
```

Named colors and `#RRGGBB` are supported.

## Development

```bash
cargo fmt -- --check
cargo test
cargo build --release --locked
cargo clippy --all-targets -- -D warnings
```

## License

MIT — see [LICENSE](LICENSE). The license file also records the extrakto lineage acknowledgement.
