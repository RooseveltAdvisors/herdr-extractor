# herdr-extractor

A [Herdr](https://herdr.dev) plugin that lists copy-eligible tokens from the focused pane's
**visible** buffer and filters them with typeahead.

## Lineage and credit

This workflow follows [laktak/extrakto](https://github.com/laktak/extrakto), “quickly select,
copy/insert/complete text without a mouse.” This Herdr port is not presented as an original UX
invention. It adapts extrakto's token-picking lineage to Herdr's visible-buffer and OSC 52 APIs.

## Action

`RooseveltAdvisors.herdr-extractor.extract` opens the `extract` overlay entrypoint.

1. The plugin calls `pane.read` with `source = "visible"` and reads the pane layout width.
2. Full-width terminal rows are rejoined as soft wraps; shorter rows retain hard newlines.
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
description = "extract a visible token"
```

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
