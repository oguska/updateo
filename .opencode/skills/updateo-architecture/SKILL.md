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
`tui-popup` 0.7, `tui-big-text` 0.8, `tui-scrollbar` 0.2, rusqlite (bundled),
serde/serde_json, chrono.

| Path | Responsibility |
|------|----------------|
| `src/main.rs` | CLI dispatch (`check [--notify]`, `update --yes`, `configure`, timer/autostart, `--version`, default TUI), actionable notification + terminal launch, terminal setup, panic hook. |
| `src/app.rs` | `App` state, tabs, async checks/search, key handling, all rendering, filter/search/details/apply/install/remove popups, layout/truncation helpers. |
| `src/config.rs` | `Config` at `$XDG_CONFIG_HOME/updateo/config.json`: notify, unattended, check_time, autostart; validation/normalization and save. |
| `src/autostart.rs` | XDG autostart `.desktop` (`check --notify` at login), install/uninstall. |
| `src/sources.rs` | `Source`/`Update`/`SearchHit`, check/search/detail commands and parsers, command builders, `UPDATEO_SUDO`, name validation, removal plans, installed versions, previews. |
| `src/history.rs` | SQLite history at `$XDG_DATA_HOME/updateo/history.db`; one-time `history.jsonl` import; `record`, `load`, `is_success`. |
| `src/runner.rs` | `run_suspended`: leaves raw mode/alternate screen, runs a child on the real TTY, restores the TUI. |
| `src/i18n.rs` | `tr(en, tr)`; language from `LC_ALL` → `LC_MESSAGES` → `LANG`; `tr` prefix means Turkish, everything else English. |
| `src/timer.rs` | systemd-user service/timer text from config (daily time, `check` vs `update --yes`) at `$XDG_CONFIG_HOME/systemd/user`, and `systemctl --user` install/uninstall. |
| `.github/workflows/ci.yml` | Push/PR checks: locked tests, clippy, release build, PTY probes. |
| `.github/workflows/release.yml` | `v*` tags: verify tag/version/changelog, test, build static musl binary, package, checksum, publish release. |

## Data flow

1. `main::run_tui` creates `App`, calls `App::start_refresh`, then loops:
   if `app.take_redraw()` is true, resize the terminal to its current area
   (this clears the screen and resets ratatui's back buffer); then
   `terminal.draw(|f| app.draw(f))` and `app.handle_events()`.
2. `spawn_checks` starts one thread per `Source::ALL` and sends `CheckMsg`
   through an mpsc channel; `drain` collects results, lowers `checking`, and
   sorts with `sources::sort_updates`. Search uses a second channel
   (`SearchMsg` + `SearchState.id`) so stale responses are dropped.
3. Tabs are `0 = pacman`, `1 = AUR`, `2 = Flatpak`, `3 = history`
   (`TAB_COUNT = Source::ALL.len() + 1`). `tab_items` maps tab-local cursor
   positions to global `updates` indexes and applies the `/` filter.
4. `handle_events` polls for 120 ms, then drains checks and search. Key
   priority is: removal → apply → details → search → filter input → package
   input → normal list keys.
5. Normal keys: `j/k`, arrows, PageUp/Down, Home/End move; `space` toggles;
   `a` toggles the tab; `r` rechecks; `R` runs `pacman -Sy` then rechecks;
   `/` filters; `s` searches; `d` opens details; `u`/Enter opens the apply
   confirmation; `i` installs; `x`/`X` removes; `q`/Esc quits. Esc clears a
   non-empty filter before it quits.
6. `run_suspended` is used for every command that may need sudo/polkit or an
   interactive package-manager flow. It sets `App::force_redraw`; the main loop
   consumes that flag and resets ratatui's buffer so tabs, hints, and borders
   are repainted after the alternate screen is restored. History is recorded
   before `spawn_checks`.
7. `cmd_check` prints per-source counts. With `--notify` and updates present it
   runs `notify-send` with a single `update` action (45 s timeout, `-w`).
   Selecting the action returns `update` on stdout; `launch_tui` then opens the
   current executable in `$UPDATEO_TERMINAL` or the first available terminal
   (`xdg-terminal-exec`, `konsole`, `alacritty`, `kitty`, `foot`, `wezterm`,
   `gnome-terminal`, `xfce4-terminal`, `xterm`). `terminal_args` owns the
   per-terminal `-e`/`--`/`start --` differences.
8. `cmd_update --yes` checks each source, applies via
   `unattended_update_command`, records history, and optionally notifies. The
   timer chooses `check --notify` or `update --yes --notify` from
   `Config.unattended`; `cmd_check`/`cmd_update` gate notifications on
   `Config.notify`.
9. `Config` is loaded by `check`, `update`, `timer::install`, and autostart
   setup. `configure` writes it with validation; `bootstrap.sh` prompts, builds,
   installs, then calls `configure`, `install-timer`, and
   `install-autostart`/`uninstall-autostart`.

## Command builders (keep them in `sources.rs`)

- Root commands: `root_command(program, args)` → `UPDATEO_SUDO` (default
  `sudo`; whitespace-separated prefix args supported, e.g. `doas -n`).
- Checks: pacman uses `checkupdates --nocolor`, falling back to `pacman -Qu`;
  AUR uses `yay -Qua`; Flatpak uses `flatpak remote-ls --updates
  --columns=app,name,version` plus `flatpak list --columns=app,version`.
- Apply/update: pacman `pacman -Syu <names>`, AUR `yay -S <names>`, Flatpak
  `flatpak update <names>`. These remain interactive by design.
- Unattended (`update --yes` / timer): pacman `pacman -Syu --noconfirm`, AUR
  `yay -S --noconfirm <names>`, Flatpak `flatpak update -y <names>`. It runs
  with stdin null and needs passwordless sudo/polkit.
- Install (`i`): pacman `pacman -S --noconfirm <names>`, AUR
  `yay -S --noconfirm <names>`, Flatpak `flatpak install -y <names>`.
  The user must not have to answer the `Y` install prompt; sudo/polkit may still
  ask for a password.
- Remove: pacman/AUR `pacman -Rns <names>`, Flatpak `flatpak uninstall <names>`.
  The in-app removal popup is the confirmation step.
- Search: pacman `pacman -Ss <query>`, AUR `yay -Ss <query>`, Flatpak
  `flatpak search <query> --columns=application,name,version,description`.
  Queries are single args and reject a leading `-`.
- Details: pacman `-Qi`, or `-Si repo/name` when a repo is known, falling
  back to `-Qi`; AUR `yay -Si`, falling back to `pacman -Qi`; Flatpak
  `flatpak info`.
- Update previews (`install_preview`, `remove_preview`, `apply_preview`) must
  match the real command and are shown on the input line / popups.

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
  `parse_reverse_deps`, `parse_search_lines`, `parse_flatpak_search`). Add tests
  there rather than testing `Command` output.
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
- The apply, search, and details popups use `KnownSizeWrapper`; only one modal
  owns keys at a time (see priority above). Details is a right sidebar at
  >=84 columns when search is closed, otherwise a popup.
- Search typing edits the query, Up/Down navigate hits, Enter searches when the
  query changed and otherwise installs the selected hit; Ctrl+D opens details.
- The list/history scrollbar uses `tui-scrollbar` in a one-column area; the
  hero title uses `tui-big-text` only at >=64x18 so small terminals keep their
  list height.
- Status text shows a braille spinner, `done/total`, and an ETA while checks
  run; keep user-facing status messages durable when a post-action recheck can
  overwrite them.
- Backend widgets are `Block`/`Paragraph`; modals use `tui_popup::Popup` with
  `KnownSizeWrapper`, and the list/history use `tui-scrollbar`.

## Tests

Unit tests live at the bottom of each module. `app.rs` tests use the
`target`/`session` helpers to build `RemovalSession` values without touching the
system. Prefer pure helpers for layout, parsing, selection, and command args.
End-to-end PTY probes live in `.e2e/`; see the `updateo-verification` skill.
