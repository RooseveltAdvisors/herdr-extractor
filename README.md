# herdr-extractor

A [Herdr](https://herdr.dev) plugin that lists copy-eligible tokens from the focused pane's
retained **scrollback** and filters them with typeahead.

## Lineage and credit

This workflow follows [laktak/extrakto](https://github.com/laktak/extrakto), “quickly select,
copy/insert/complete text without a mouse.” This Herdr port is not presented as an original UX
invention. It adapts extrakto's token-picking lineage to Herdr's scrollback and OSC 52 APIs.

## Actions

One action, one picker, two data modes:

| Action | Default key | Reads |
| --- | --- | --- |
| `RooseveltAdvisors.herdr-extractor.extract` | `prefix+space` | Retained pane scrollback; press `Ctrl+G` inside the picker to switch to the full saved session history |
| `RooseveltAdvisors.herdr-extractor.extract_transcript` | optional (for example `prefix+shift+space`) | The pane's full retained session transcript, opening the picker straight in global mode |

### In-menu mode toggle (`Ctrl+G`)

The `prefix+space` extract picker has a mode toggle built in. Press `Ctrl+G` inside the overlay to
switch between the two data modes:

- **scrollback mode** (`mode:scrollback` in the status line): the focused pane's retained
  scrollback, as always.
- **global mode** (`mode:global`): the pane's full saved session history, the same transcript-grade
  coverage the `extract_transcript` action provides. When Herdr saves pane history
  (`experimental.pane_history = true`, default off) this reads every retained line from
  `session-history.json`; otherwise it falls back to the server-capped 1000-line
  `recent_unwrapped` transcript and the coverage note explains the limit.

Toggling re-extracts live and keeps the current filter query and selection where possible. The
active mode is always visible in the status line, and the hint text names the toggle
(`ctrl-g:mode`).

The picker also protects its rendering from Herdr overlay geometry drift: it uses the live pane
layout as the drawable boundary, so the status row and token text remain inside a narrower or
shorter overlay. When Herdr reports the normal full pane size, rendering is unchanged.

### Scrollback extract (`prefix+space`)

`RooseveltAdvisors.herdr-extractor.extract` opens the `extract` overlay entrypoint.

1. The plugin calls `pane.read` with `source = "recent_unwrapped"` and requests the maximum line
   bound when that parameter is supported, so text found while reading copy mode remains available
   after returning to normal mode. Older Herdr versions fall back to `recent`, then `visible`, only
   when a source is unsupported.
2. Herdr supplies logical lines for `recent_unwrapped`; fallback sources use the pane layout width.
3. A bounded extrakto-parity set collects URLs, paths, double/single quotes, and words of at least
   five characters. Lower/recent results come first and duplicates are removed.
4. Type to filter. `Up`/`Down` or `Ctrl-p`/`Ctrl-n` moves selection. `Ctrl+G` toggles between
   scrollback and global (full session history) modes. `Enter` copies exactly one item
   through OSC 52. `Esc` or `Ctrl-C` cancels.

### Session transcript extract (`extract_transcript`)

`RooseveltAdvisors.herdr-extractor.extract_transcript` opens the `extract-transcript` overlay
entrypoint and reads the whole retained session transcript, not just the current viewport. It
starts the picker straight in global mode; inside the picker it behaves exactly like the
`prefix+space` extract with `Ctrl+G` already applied. Keep the binding only if you want a
shortcut that lands directly in global mode - the in-menu toggle covers the same data.

- The transcript source covers the whole retained session. When Herdr saves pane history
  (`experimental.pane_history = true` in the herdr config, default off), the extractor reads the
  pane's full saved transcript from `session-history.json` in the session data directory - every
  retained line, including content older than the API cap. Otherwise it reads `pane.read` with
  `source = "recent_unwrapped"`; the server caps that at 1000 logical lines, so a transcript log
  records a `transcript_note` naming the covered line count and, if toasts are enabled, raises a
  notification saying `experimental.pane_history = true` unlocks full-session coverage. Retention
  itself is bounded by `advanced.scrollback_limit_bytes`.
- The transcript read never degrades to viewport-shaped sources. On a Herdr without
  `recent_unwrapped` it fails instead of silently pretending the viewport is the session.
- Herdr alt-screen panes do not keep host scrollback. When `pane.get` reports no retained history
  (`scroll.max_offset_from_bottom = 0`), the picker still opens with the viewport content but logs
  `transcript_note` and, if toasts are enabled, raises a notification saying the pane keeps no host
  scrollback. It never presents viewport-only results as the session transcript.

Typeahead, selection, and OSC 52 copy behave exactly like the scrollback extract.

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

# Optional: land directly in global (full session history) mode. The Ctrl+G
# toggle inside the extract picker reaches the same data without this binding.
[[keys.command]]
key = "prefix+shift+space"
type = "plugin_action"
command = "RooseveltAdvisors.herdr-extractor.extract_transcript"
description = "extract a token from the session transcript"
```

Copy mode is for READING; `prefix+space` is for TAKING. Scroll through pane output with copy mode,
exit to normal mode, then invoke `RooseveltAdvisors.herdr-extractor.extract` with `prefix+space`.
The picker searches retained scrollback and copies the chosen result through OSC 52, so it reaches
the outer terminal clipboard (including the captain's Mac). When the token may sit far above the
current viewport, press `Ctrl+G` inside the picker to switch to global mode and search the pane's
full saved session history.

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
