---
name: updateo-architecture
description: Navigate and modify the updateo Rust TUI: module responsibilities, check/install/remove data flow, key handling, command builders, i18n, history, and test conventions. Use when changing updateo features, UI behavior, package-manager integration, or tests.
metadata:
  audience: developers
  workflow: implementation
  opencode/autoinvoke: "true"
---

## When to use

Use this before editing updateo. It maps the code so a change lands in the
right module and follows the existing conventions instead of adding parallel
logic.

## Stack and layout

Rust 2024, ratatui 0.30.2, crossterm 0.29, `tui-input` 0.15,
`tui-popup` 0.7, rusqlite (bundled), serde/serde_json, chrono.

| Path | Responsibility |
|------|----------------|
| `src/main.rs` | CLI dispatch (`check [--notify]`, `install-timer`, `uninstall-timer`, default TUI), terminal setup, panic hook. |
| `src/app.rs` | `App` state, tabs, async checks, key handling, all rendering, install/remove input modes, removal popup, layout/truncation helpers. |
| `src/sources.rs` | `Source`/`Update`, check commands and parsers, command builders, `UPDATEO_SUDO`, name validation, removal plans, installed versions, previews. |
| `src/history.rs` | SQLite history at `$XDG_DATA_HOME/updateo/history.db`; one-time `history.jsonl` import; `record`, `load`, `is_success`. |
| `src/runner.rs` | `run_suspended`: leaves raw mode/alternate screen, runs a child on the real TTY, restores the TUI. |
| `src/i18n.rs` | `tr(en, tr)`; language from `LC_ALL` → `LC_MESSAGES` → `LANG`; `tr` prefix means Turkish, everything else English. |
| `src/timer.rs` | systemd-user service/timer text and `systemctl --user` install/uninstall. |

## Data flow

1. `main::run_tui` creates `App`, calls `App::start_refresh`, then loops
   `terminal.draw(|f| app.draw(f))` and `app.handle_events()`.
2. `spawn_checks` starts one thread per `Source::ALL` and sends `CheckMsg`
   through an mpsc channel; `drain` collects results, lowers `checking`, and
   sorts with `sources::sort_updates`.
3. Tabs are `0 = pacman`, `1 = AUR`, `2 = Flatpak`, `3 = history`
   (`TAB_COUNT = Source::ALL.len() + 1`). `tab_items` maps tab-local cursor
   positions to global `updates` indexes.
4. `handle_events` polls for 100 ms. Priority for keys is:
   removal popup → package input line → normal list keys.
5. Normal keys: `j/k`, arrows, PageUp/Down, Home/End move; `space` toggles;
   `a` toggles the tab; `r` rechecks; `R` runs `pacman -Sy` then rechecks;
   `u`/Enter applies selected updates; `i` installs; `x`/`X` removes; `q`/Esc quits.
6. `run_suspended` is used for every command that may need sudo/polkit or an
   interactive package-manager flow. The TUI is restored afterward and history
   is recorded before `spawn_checks`.

## Command builders (keep them in `sources.rs`)

- Root commands: `root_command(program, args)` → `UPDATEO_SUDO` (default
  `sudo`; whitespace-separated prefix args supported, e.g. `doas -n`).
- Checks: pacman uses `checkupdates --nocolor`, falling back to `pacman -Qu`;
  AUR uses `yay -Qua`; Flatpak uses `flatpak remote-ls --updates
  --columns=app,name,version` plus `flatpak list --columns=app,version`.
- Apply/update: pacman `pacman -Syu <names>`, AUR `yay -S <names>`, Flatpak
  `flatpak update <names>`. These remain interactive by design.
- Install (`i`): pacman `pacman -S --noconfirm <names>`, AUR
  `yay -S --noconfirm <names>`, Flatpak `flatpak install -y <names>`.
  The user must not have to answer the `Y` install prompt; sudo/polkit may still
  ask for a password.
- Remove: pacman/AUR `pacman -Rns <names>`, Flatpak `flatpak uninstall <names>`.
  The in-app removal popup is the confirmation step.
- Update previews (`install_preview`, `remove_preview`) must match the real
  command and are shown on the input line / removal footer.

## Safety and parsing rules

- `split_names` trims, de-duplicates while preserving order, rejects empty
  input, and rejects any name starting with `-` to prevent option injection.
  Keep that invariant if you add new command paths.
- `PROTECTED` (`base`, `base-devel`, `filesystem`, `pacman`, `libalpm`,
  `mkinitcpio`, `systemd`) and not-installed targets are blocked in the removal
  plan and cannot be toggled.
- `removal_plan` gathers installed version, reverse dependencies
  (`pactree -r --depth=1`) or Flatpak runtime users, notes, and block reasons.
- Parsers are pure and unit-tested (`parse_arrow_lines`, `parse_flatpak_lines`,
  `parse_q_version`, `parse_ref_version`, `parse_runtime_users`,
  `parse_reverse_deps`). Add tests there rather than testing `Command` output.
- `capture` treats exit 1/2 with empty stderr as the “no updates” convention.

## UI conventions

- Every user-visible string goes through `tr("English", "Türkçe")`; keep both
  sides meaningful and test Turkish text when layout matters.
- Use character counts (`chars().count()`/`truncate`) not byte lengths for
  terminal widths; Turkish characters must fit.
- `truncate_tail` preserves the runnable command tail; `compact_wrapper` drops
  only the directory path of a wrapper; `command_preview` and `input_layout`
  share width budget between label, preview, input, and hint.
- `draw_removal` keeps counts at the top and the command preview pinned at the
  bottom; the list scrolls between them. Keep this invariant for small terminals.
- Backend widgets are `Block`/`Paragraph`; the removal dialog uses
  `tui_popup::Popup` with `KnownSizeWrapper`.

## Tests

Unit tests live at the bottom of each module. `app.rs` tests use the
`target`/`session` helpers to build `RemovalSession` values without touching the
system. Prefer pure helpers for layout, parsing, selection, and command args.
End-to-end PTY probes live in `.e2e/`; see the `updateo-verification` skill.
