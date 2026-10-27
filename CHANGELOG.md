# Changelog

All notable changes to updateo are documented here. The project follows
[Semantic Versioning](https://semver.org/): the version is defined once in
`Cargo.toml` and exposed by `updateo --version`.

## 0.2.1 — 2026-10-27

- First tagged release: GitHub Actions CI (tests, clippy, release build, PTY
  probes) and a tag-driven release workflow that publishes a static musl
  binary and checksums.
- Add `bootstrap.sh`, an interactive installer that asks about notifications,
  unattended updates, the daily check time, and the login check.
- Add `$XDG_CONFIG_HOME/updateo/config.json` for notification, unattended
  update, check time, and autostart preferences.
- Add `updateo update --yes [--notify]` for systemd-driven unattended updates.
- Make the systemd timer use the configured check time and switch between
  `check` and unattended `update`.
- Add `install-autostart` / `uninstall-autostart` XDG autostart entries.
- Add `--version` and show the version in the TUI title.
- Add English documentation in `README.en.md`.
- Fix the TUI repaint after update/install/removal: reset ratatui's buffer when
  returning from a suspended command so tabs, hints, and list borders remain.

## 0.1.0

- Initial TUI with pacman, AUR (yay), and Flatpak update checks.
- Selection, apply confirmation, install, removal with dependency warnings,
  history, search/details, live filtering, and actionable desktop
  notifications.
