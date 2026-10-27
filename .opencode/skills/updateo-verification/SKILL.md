---
name: updateo-verification
description: Verify updateo changes with cargo test, clippy, a release build, and the PTY-based .e2e probe suite. Use after implementation, before claiming completion, or when asked to check for errors.
metadata:
  audience: developers
  workflow: validation
  opencode/autoinvoke: "true"
---

## When to use

Use after any code change in updateo and before reporting success. The e2e
suite runs the real release binary in a PTY, so a passing `cargo test` alone is
not enough for TUI or command-building changes.

## Check order

Run from the repository root:

```sh
cargo test --all-targets
cargo clippy --all-targets -- -D warnings
cargo build --release
sh .e2e/probes.sh
```

- `cargo build --release` is required: `.e2e/probes.sh` executes
  `target/release/updateo`, not the debug/test binary. Rebuild after source
  changes or the probes test stale code.
- `probes.sh` needs `python3` and finishes with `ALL PROBES OK` on success.
- The probes are deterministic: fake package managers from `.e2e/tools` plus
  `.e2e/fake_sudo`, and a fake database at `.e2e/fake_db`.
- Do not run the probes against the live system. `screen_dump.py` overrides
  `PATH`, `UPDATEO_SUDO`, `UPDATEO_FAKE_DB`, `FAKE_SUDO_LOG`, and
  `XDG_DATA_HOME`; the fake sudo only appends to `.e2e/sudo.log`.

## Formatting

The repository is not fully rustfmt-clean: `app.rs` has pre-existing formatting
diffs in its test assertions. Do **not** run `cargo fmt --all` or mass-format
files, because it rewrites unrelated code. For a file you touched, use:

```sh
rustfmt --check --edition 2024 src/<file>.rs
rustfmt --edition 2024 src/<file>.rs   # only when the diff is yours
```

## What the e2e probes cover

`sh .e2e/probes.sh` runs these cases and prints one line per probe:

1. Input line keeps typed names (`x nano sed`) and preview is readable.
2. A long `UPDATEO_SUDO` path still leaves the typed names visible.
3. English removal dialog: blocked targets/reasons, reverse deps, counts,
   command footer.
4. Turkish removal dialog text.
5. Option-like name (`-Rns`) is refused inline and never reaches sudo, in both
   English and Turkish.
6. Confirming removal (`y`) sends the full command to the wrapper and writes
   history (`removed`, target names).
7. Cancelling with Esc runs nothing and leaves `sudo.log` empty.
8. Installing (`i nano`) reaches the wrapper as
   `pacman -S --noconfirm nano`, proving the Y prompt is skipped.

The harness is `.e2e/screen_dump.py`:

```sh
python3 .e2e/screen_dump.py <XDG_DATA_HOME> "<keys>" \
  [--expect TEXT]... [--log TEXT]... [--input-cell TEXT] \
  [--width N] [--height N] [--sudo PATH]
```

Key syntax: literal characters plus `<CR>`, `<BS>`, `<ESC>`, `<UP>`, `<DOWN>`,
`<TAB>`, `<SP>`, `<BACKTAB>`. Keys are sent 150 ms apart. Use `--expect` for
rendered text, `--log` for exact fake-command lines, and `--input-cell` only
when the input line is visible (it looks for the `Enter/Esc` hint).

When adding a behavior probe:

- Prefer `--log` for command/flag assertions and `--expect` for user-facing text.
- The post-action `spawn_checks` can overwrite the status line within a few
  ticks; assert durable evidence in history/the log instead of a transient
  status message.
- Keep probes independent. Each run truncates `.e2e/sudo.log` and the
  `.e2e/logs/*` files.
- If a fake command needs a new subcommand, add it under `.e2e/tools/` and keep
  it side-effect-free.

## Reporting

Report the exact commands run and their results. Separate pre-existing failures
(for example rustfmt diffs in untouched code) from failures caused by the
change. State skipped checks and residual risk explicitly; never claim a check
passed without its output.
