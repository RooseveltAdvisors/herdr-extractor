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
| `RooseveltAdvisors.herdr-extractor.extract` | `prefix+space` | Retained pane scrollback; press `Tab` inside the picker to switch to the full saved session history |
| `RooseveltAdvisors.herdr-extractor.extract_transcript` | optional (for example `prefix+shift+space`) | The pane's full retained session transcript, opening the picker straight in global mode |

### In-menu mode toggle (`Tab`)

The `prefix+space` extract picker has a mode toggle built in. Press `Tab` inside the overlay to
switch between the two data modes:

- **scrollback mode** (`SCROLLBACK` in the header): the focused pane's retained
  scrollback, as always.
- **global mode** (`GLOBAL` in the header): the pane's full saved session history, the same transcript-grade
  coverage the `extract_transcript` action provides. When Herdr saves pane history
  (`experimental.pane_history = true`, default off) this reads every retained line from
  `session-history.json`; otherwise it falls back to the server-capped 1000-line
  `recent_unwrapped` transcript and the coverage note explains the limit.

Toggling re-extracts live and keeps the current filter query and selection where possible. The
active mode is always visible in the top header, and the hint text names the toggle
(`tab mode`).

The picker uses a dense gh-dash-inspired layout: a colored top header shows the mode and active
engine badge (`SCROLLBACK:REGEX`, `GLOBAL:NLP`), match count, and `tab mode · enter copy · esc
cancel` hints; each row has a semantic color, compact kind chip, and Nerd-Font glyph (with ASCII
fallback); a prompt line shows the live query and a wrapped detail area shows the complete selected
item; the list highlights matched characters. The header is intentionally top-anchored so
all captain-facing status survives Herdr's measured bottom overlay chrome. Three bottom pane rows
are reserved as decoration, controlled by the documented `RESERVED_BOTTOM_ROWS` constant.

Matching is implemented in-house with the existing ratatui/crossterm stack: smart-case
subsequence matching scores contiguous runs and word-boundary starts, and adds no external `fzf`
binary dependency. Keeping it in the plugin preserves a single static binary and lets Tab trigger
live mode re-extraction directly.

### Scrollback extract (`prefix+space`)

`RooseveltAdvisors.herdr-extractor.extract` opens the `extract` overlay entrypoint.

1. The plugin calls `pane.read` with `source = "recent_unwrapped"` and requests the maximum line
   bound when that parameter is supported, so text found while reading copy mode remains available
   after returning to normal mode. Older Herdr versions fall back to `recent`, then `visible`, only
   when a source is unsupported.
2. Herdr supplies logical lines for `recent_unwrapped`; fallback sources use the pane layout width.
3. A bounded extrakto-parity set collects URLs, paths, double/single quotes, and words of at least
   five characters. It also recognizes commands, hashes, versions, error lines, and backtick/JSON
   code. ANSI/SGR residue, box fragments, ratios, percentages, punctuation tails, and wrapped
   slivers are discarded. Canonicalized duplicates collapse to one item, ranked by recency,
   semantic usefulness, length, and character variety.
4. Type to filter. `Up`/`Down` or `Ctrl-p`/`Ctrl-n` moves selection. `Tab` toggles between
   scrollback and global (full session history) modes. `Enter` copies exactly one item
   through OSC 52. `Esc` or `Ctrl-C` cancels.

### Session transcript extract (`extract_transcript`)

`RooseveltAdvisors.herdr-extractor.extract_transcript` opens the `extract-transcript` overlay
entrypoint and reads the whole retained session transcript, not just the current viewport. It
starts the picker straight in global mode; inside the picker it behaves exactly like the
`prefix+space` extract with `Tab` already applied. Keep the binding only if you want a
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

# Optional: land directly in global (full session history) mode. The Tab
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
current viewport, press `Tab` inside the picker to switch to global mode and search the pane's
full saved session history.

This action moved out of `RooseveltAdvisors.herdr-leap` in the public plugin split. Do not bind
`prefix+space` to `RooseveltAdvisors.herdr-leap.open`; that opens the separate jump workflow.

The launcher validates `HERDR_BIN_PATH` and falls back to `herdr` on `PATH`, covering a replaced
Linux server executable whose stale path ends in ` (deleted)`.

## Configuration

Create `config.toml` under `herdr plugin config-dir RooseveltAdvisors.herdr-extractor`:

```toml
copy_toast = true
icons = false

[style]
selected_match_fg = "cyan"
selected_match_bg = "dark-gray"
status_bg = "gray"
url_fg = "gray"
path_fg = "cyan"
error_fg = "gray"
command_fg = "gray"
hash_fg = "gray"
version_fg = "gray"
quote_fg = "dark-gray"
code_fg = "gray"

[nlp]
# Optional, off by default. Regex extraction remains the instant fallback.
enabled = false
socket_path = "/run/user/1000/herdr-extractor-nlp.sock"
confidence_threshold = 0.75
```

Named colors and `#RRGGBB` are supported. The existing style settings remain valid; the semantic
foreground settings only affect unselected rows. `icons` may also be set through the environment
with `HERDR_EXTRACTOR_ASCII_ICONS=1`.

### Optional NLP sidecar contract

NLP mode never bundles a model. When `[nlp].enabled = true`, the plugin connects to the configured
Unix socket for each extraction with a 180ms read/write deadline. If `socket_path` is omitted it
uses `HERDR_NLP_SOCKET_PATH`, then `nlp.sock` under the plugin config directory. A connection,
timeout, or protocol error is logged and the normal regex list is used immediately. Tab mode
switches repeat the same bounded request, and the header reports `NLP` only after a request
succeeds.

The protocol is one JSON request and one JSON response per connection:

```json
{"version":1,"text":"cargo test --release"}
```

```json
{"version":1,"candidates":[
  {"text":"cargo test --release","kind":"command","confidence":0.98},
  {"text":"--release","kind":"command","confidence":0.91}
]}
```

`kind` is one of the stable keys `url`, `path`, `quote`, `squote`, `word`, `command`, `hash`,
`version`, `error`, or `code`. Candidate text must be at least five characters. Candidates below
the configured confidence threshold are ignored; accepted candidates replace the regex item's
kind when their canonical text ties. Any local runtime can implement this contract. For example,
a Python sidecar can read stdin from a Unix socket, call a locally installed model or rules engine,
and write the response JSON followed by a newline. The model remains outside this static plugin.

## Development

```bash
cargo fmt -- --check
cargo test
cargo build --release --locked
cargo clippy --all-targets -- -D warnings
```

## License

MIT — see [LICENSE](LICENSE). The license file also records the extrakto lineage acknowledgement.
