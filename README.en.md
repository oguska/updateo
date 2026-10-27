# updateo

[![CI](https://github.com/oguska/updateo/actions/workflows/ci.yml/badge.svg)](https://github.com/oguska/updateo/actions/workflows/ci.yml)
[![Release](https://github.com/oguska/updateo/actions/workflows/release.yml/badge.svg)](https://github.com/oguska/updateo/actions/workflows/release.yml)

English · [Türkçe](README.md)

A single-interface update manager for Arch-based systems (Arch, CachyOS,
Manjaro…): it lists **pacman (repos) + AUR (yay) + Flatpak** updates in one
TUI, lets you pick packages individually, and applies them directly.

## Features

- Three tabs: `pacman`, `AUR`, `Flatpak`, each with a selected/total badge
- Flatpak applications are separated from runtime/extension updates: runtimes
  come last, carry a `(runtime)` label, and show the installed version
  (`old -> new`)
- Fourth `history` tab: timestamped log of applied updates
  (`~/.local/share/updateo/history.db` — SQLite; the old `history.jsonl` is
  imported once on first start)
- Per-package selection (`space`), select/clear all in the tab (`a`)
- `/` live filter for the current tab: narrows name, display name, and version
  fields immediately; Enter keeps the filter, Esc clears it
- `s` package search: `pacman -Ss` / `yay -Ss` / `flatpak search` results in a
  popup; Enter opens details, Ctrl+I installs the selected result
- `d` package details: side panel on wide terminals, popup on narrow ones
  (`pacman -Qi/-Si`, `yay -Si`, `flatpak info`)
- Visual refresh: `tui-scrollbar` on the list/history, big-text title on wide
  terminals, and a braille spinner + `done/total` + ETA while checking
- `u`/`Enter` apply: a per-source plan popup opens first (`y`/Enter applies);
  the TUI is then suspended and `sudo pacman -Syu`, `yay -S`, and
  `flatpak update` run in the real TTY with all interactive prompts
  (password, pacman conflict questions, yay build flow)
- Single-package install: `i` opens a name input in the status bar; Enter runs
  the current tab's installer (`sudo pacman -S --noconfirm` / `yay -S
  --noconfirm` / `flatpak install -y`) in the real TTY — no Y confirmation,
  only a possible sudo password; the result is written to history
- Checks run in parallel threads; a network failure only drops one source and
  never blocks the UI
- Actionable desktop notification: when updates are available, an **Update
  now** button opens the TUI in a desktop terminal; if `notify-send` is
  missing, the message goes to the journal
- `~/.config/updateo/config.json` persists notifications, unattended updates,
  the daily check time, and the login check
- Single static binary, no runtime dependency (SQLite is embedded)
- UI language follows the system locale: `LC_ALL` → `LC_MESSAGES` → `LANG`;
  a `tr` prefix selects Turkish, everything else English

## Commands

```sh
updateo                        TUI
updateo check [--notify]       headless check; --notify shows an actionable notification
updateo update --yes [--notify]  apply all updates without prompts (automation)
updateo configure [--notify yes|no] [--unattended yes|no]
                  [--check-time HH:MM] [--autostart yes|no]
updateo install-timer          systemd-user daily timer (configured time)
updateo uninstall-timer        remove the timer
updateo install-autostart      install the login background check
updateo uninstall-autostart    remove the login check
updateo --version              print the version
```

Once the timer is installed it runs a daily `check --notify`; if updates are
available, `notify-send` shows a notification (falling back to the systemd
journal). Its **Update now** button opens the TUI; override the terminal with
`UPDATEO_TERMINAL`. If `unattended` is enabled, the timer runs `update --yes
--notify` instead, which requires passwordless sudo. The check time,
notification setting, and unattended mode are read from
`~/.config/updateo/config.json`, written by `configure` or `bootstrap.sh`.

## Keys

```
j / k / ↑ / ↓   move             PageUp/PageDown  jump 10
space           select           a                select all in tab
1 / 2 / 3 / 4   tabs (4 = history)   Tab / arrow keys  switch tab
r               re-check         R                refresh db (sudo pacman -Sy)
/               live filter (Enter keeps, Esc clears)
s               package search (type, Enter search/details, ↑/↓ select,
                                 Ctrl+I install, Esc close)
d               details panel (j/k scroll, d/Esc close)
u / Enter       update selected packages (plan confirmation first; y/Enter applies)
q / Esc         quit
i               install one package (sudo pacman -S --noconfirm /
                yay -S --noconfirm / flatpak install -y; no Y prompt,
                Enter starts, Esc cancels)
x               type package name(s) to remove (Enter opens confirmation)
X               target selected rows (or the cursor row) for removal
```

The filter only narrows the view; selections on hidden rows are preserved. The
query in the search popup runs through `pacman -Ss` / `yay -Ss` / `flatpak
search`; `Enter` searches when the query changed, otherwise it opens details
for the selected result. `Ctrl+I` installs the selected result with its actual
source's installer.

The removal confirmation popup lists the installed version, `pactree -r`
reverse dependencies (Flatpak apps using that runtime), warnings, and blocked
reasons for every target; protected packages such as `base`, `base-devel`,
`filesystem`, `pacman`, `mkinitcpio`, and `systemd`, plus names that are not
installed, cannot be selected. Counts stay pinned at the top and the command
runs at the bottom; `space` selects, `a` selects all, `y` removes (the TUI is
suspended and `sudo pacman -Rns` / `flatpak uninstall` runs in the real TTY
with its password/prompts), `Esc` cancels. The result is written to history.

## Build

```sh
cargo build --release
./target/release/updateo
```

Install: `sudo install -Dm755 target/release/updateo /usr/local/bin/updateo`

## Bootstrap

`./bootstrap.sh` asks four questions, builds the release binary, installs it to
`/usr/local/bin/updateo`, writes `config.json`, and enables the systemd timer
and login check:

1. Show desktop notifications?
2. Apply updates unattended? (requires sudo NOPASSWD)
3. Daily check time (default `09:30`)
4. Run a background check at login?

For non-interactive installs, use environment variables:

```sh
UPDATEO_ASSUME_YES=1 UPDATEO_NOTIFY=yes UPDATEO_UNATTENDED=no \
UPDATEO_CHECK_TIME=09:30 UPDATEO_AUTOSTART=yes ./bootstrap.sh
```

Set `UPDATEO_BINDIR` to change the install directory. The version comes from
`package.version` in `Cargo.toml`; `updateo --version` and the TUI title show
the same value. Changes are recorded in `CHANGELOG.md`.

## CI and releases

Every push to `main` and every pull request runs the `CI` workflow: unit tests,
clippy with warnings denied, a release build, and the PTY end-to-end probes.

Pushing a `v*` tag runs the `Release` workflow. The tag must match
`package.version` in `Cargo.toml` and `CHANGELOG.md` must have a matching
section. After the tests pass, it builds a statically linked musl binary,
packages it, writes `SHA256SUMS`, and publishes a GitHub release. The release
archive contains the binary plus both READMEs and the changelog:

```sh
tar -xzf updateo-<version>-x86_64-unknown-linux-musl.tar.gz
sudo install -Dm755 updateo-<version>-x86_64-unknown-linux-musl/updateo /usr/local/bin/updateo
```

To publish a version, update the version in `Cargo.toml` and add its section to
`CHANGELOG.md`, then tag the commit:

```sh
git tag v0.3.0
git push origin v0.3.0
```

## How it works

| Source  | Check                            | Apply                    | Search / details |
|---------|----------------------------------|--------------------------|------------------|
| pacman  | `checkupdates` (pacman-contrib), fallback `pacman -Qu` | `sudo pacman -Syu <packages>` | `pacman -Ss` / `pacman -Qi`, `pacman -Si repo/name` |
| AUR     | `yay -Qua`                       | `yay -S <packages>`      | `yay -Ss` / `yay -Si` |
| Flatpak | `flatpak remote-ls --updates`    | `flatpak update <refs>`  | `flatpak search` / `flatpak info` |

Notes:
- The repo list uses cached databases; for the freshest result press `R` to run
  `sudo pacman -Sy` and check again (or use your distribution's mirror
  updater).
- Commands that need root use `sudo` by default; set
  `UPDATEO_SUDO=doas updateo` to change it (prefix arguments such as
  `UPDATEO_SUDO="doas -n"` are supported).
- AUR queries need network access (yay uses the AUR RPC API).
- `s` searches the current tab's source; when an AUR result has a repo other
  than `aur`, it is treated as a pacman package. `d`/Enter details use `-Si`,
  `-Qi`, or `flatpak info` according to the selected result's source.
- The `u` plan groups selected packages by source; the confirmation popup shows
  the command for each source. The filter only narrows the view, so selected
  packages on hidden rows are still included.
- Installs with `i` use pacman/yay `--noconfirm` and Flatpak `-y`: no install
  confirmation, only a possible sudo password.
- The notification action stays visible for 45 seconds; clicking it opens the
  TUI (sudo/pacman prompts still happen in a real terminal). If the desktop
  notification service does not support actions, the notification is
  informational only.
- With `notify=false`, `check --notify` stays silent. With `unattended=true`,
  the timer runs `update --yes --notify`; because there is no TTY, sudo must be
  passwordless (NOPASSWD or polkit). AUR builds also run unattended, so the
  option is off by default.
- `autostart=true` does not open the TUI; it runs `check --notify` in the
  background at login. The setting is written to
  `~/.config/autostart/updateo.desktop`.
- Flatpak applications may ask for polkit/password depending on the system
  installation.

## Roadmap

- [x] Refresh the db during `r` (`sudo pacman -Sy`) — `R`
- [x] Update history tab — `4`, SQLite log
- [x] Flatpak runtime/extension separation — `(runtime)` label, sorted last,
  installed version display (`flatpak list` matches the ref's first component)
- [x] Background notification with a systemd timer — `updateo install-timer` +
  `check --notify`; the notification's **Update now** button opens the TUI
- [x] Move history from JSONL to SQLite (`history.db`, automatic import)
- [x] English UI — automatic from the system locale (TR/EN)
- [x] Package install: `i` → name input (tui-input) → `pacman -S
  --noconfirm` / `yay -S --noconfirm` / `flatpak install -y` through the
  suspend-and-restore flow; result is written to history
- [x] Package removal: `x` / `X` → confirmation popup (tui-popup) +
  `pactree -r` reverse-dependency summary + `base`/`base-devel`/`systemd`
  protection + not-installed guard → `sudo pacman -Rns` / `flatpak uninstall`
- [x] Package search and `/` live filter: `pacman -Ss` / `yay -Ss` output,
  details panel (`pacman -Qi/-Si`, `flatpak info`)
- [x] Visual refresh: smooth `tui-scrollbar` scrolling, animated spinner + ETA
  during checks, popup confirmation flow, big-text title (all tested against
  ratatui 0.30.2)
- [x] Interactive bootstrap (`bootstrap.sh`), persistent `config.json`,
  configurable check time, unattended updates, and login check
- [x] Version system: `Cargo.toml` → `--version` / TUI title, `CHANGELOG.md`
- [x] English README (`README.en.md`)

## License

MIT
