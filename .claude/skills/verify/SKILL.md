---
name: verify
description: Prove herdr-extractor through a real isolated Herdr pane before the PR.
user_invocable: true
---

# /verify — prove the task before the PR

This is the independent runtime-proof layer for herdr-extractor. It complements, and never
replaces, the existing Cargo tests, clippy gate, manifest checks, or no-mistakes review and CI
gates.

## Preconditions

Use a feature branch with committed changes. Confirm `cargo`, `herdr`, and `tmux` are available.
Run the real plugin in a second Herdr instance with a scrubbed temporary `HOME`,
`XDG_CONFIG_HOME`, and `XDG_STATE_HOME`, plus a throwaway tmux socket. Follow the lab-isolation
boundary in `AGENTS.md`; never link this checkout into or invoke the plugin from the operator's
default Herdr session.

Keep captures under the ignored `evidence/` directory. Fixtures must be synthetic only: URLs,
paths, and quoted words invented for this run. Do not use real scrollback, credentials, tokens,
PHI, or production paths.

## Fresh read-only verification

Delegate this brief without the implementer's context:

> Independently verify the committed herdr-extractor plugin in the isolated Herdr lab. Build the
> real release binary with `cargo build --release --locked`, link this checkout into the lab, and
> create a disposable shell pane containing synthetic scrollback such as
> `https://verify.invalid/item`, `/tmp/herdr-extractor-proof`, and `"synthetic quote"`. Open
> `RooseveltAdvisors.herdr-extractor.extract` through the real plugin action. Capture the extractor
> pane before interaction and confirm the URL, path, and quoted value are present. Type `verify`,
> confirm the candidate list narrows to the synthetic URL, press `Enter`, and confirm the state log
> records `outcome=Copy("https://verify.invalid/item")`. Capture the pane and redacted state log
> under `evidence/`. Return exactly `TASK: works | broken`, expected, observed, and evidence paths.
> Do not edit code, use the default Herdr session, or include raw environment/config output.

The proof must exercise the real binary, plugin action, Herdr socket, terminal UI, and OSC 52 copy
path. Pane output and the redacted state log are the proof; source-text greps and unit tests alone
are not proof. If the verdict is `broken`, fix the implementation, commit it, and use a fresh lab
and verifier. Cap the fix/verify loop at three rounds, then escalate.

## Regression proof

After `works`, run all existing checks without weakening them:

```sh
cargo fmt -- --check
cargo test
cargo build --release --locked
cargo clippy --all-targets -- -D warnings
git diff --check
```

Keep evidence local and synthetic. Only after the independent runtime proof and regression sweep
pass should `/no-mistakes` run with the complete task intent; no-mistakes remains authoritative
for review, push, PR, and CI.
