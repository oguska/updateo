#!/bin/sh
# updateo bootstrap: ask for the four setup choices, install the binary, write
# the config, and enable the systemd timer / login check.
#
# Non-interactive use:
#   UPDATEO_ASSUME_YES=1 UPDATEO_NOTIFY=no UPDATEO_UNATTENDED=yes \
#   UPDATEO_CHECK_TIME=08:15 UPDATEO_AUTOSTART=yes ./bootstrap.sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
cd "$ROOT"
SUDO=${SUDO:-sudo}
BINDIR=${UPDATEO_BINDIR:-/usr/local/bin}
CONFIG_DIR=${XDG_CONFIG_HOME:-${HOME:-/tmp}/.config}/updateo
ASSUME_YES=${UPDATEO_ASSUME_YES:-0}

say() { printf '%s\n' "$*"; }

ask_yn() {
    prompt=$1
    default=$2
    if [ "$ASSUME_YES" = 1 ]; then
        case "$default" in
            y|Y) say "$prompt yes (default)" ; return 0 ;;
            *)   say "$prompt no (default)" ; return 1 ;;
        esac
    fi
    while :; do
        if [ "$default" = y ]; then
            printf '%s [Y/n] ' "$prompt"
        else
            printf '%s [y/N] ' "$prompt"
        fi
        IFS= read -r answer || answer=
        case "$answer" in
            '') [ "$default" = y ] && return 0 || return 1 ;;
            y|Y|yes|YES) return 0 ;;
            n|N|no|NO) return 1 ;;
            *) say "Please answer yes or no." ;;
        esac
    done
}

valid_time() {
    case "$1" in
        [0-1][0-9]:[0-5][0-9]|2[0-3]:[0-5][0-9]) return 0 ;;
        *) return 1 ;;
    esac
}

command -v cargo >/dev/null 2>&1 || {
    say "cargo is required to build updateo (install rustup or the 'rust' package)."
    exit 1
}

say "updateo bootstrap"
say "This will build updateo, install it to $BINDIR, and configure it."
say

# 1. Notifications
if [ -n "${UPDATEO_NOTIFY:-}" ]; then
    case "$UPDATEO_NOTIFY" in
        y|Y|yes|YES) notify=yes ;;
        *) notify=no ;;
    esac
else
    if ask_yn "Show desktop notifications when updates are available?" y; then
        notify=yes
    else
        notify=no
    fi
fi

# 2. Unattended updates
if [ -n "${UPDATEO_UNATTENDED:-}" ]; then
    case "$UPDATEO_UNATTENDED" in
        y|Y|yes|YES) unattended=yes ;;
        *) unattended=no ;;
    esac
else
    if ask_yn "Apply updates automatically (unattended, no confirmation)?" n; then
        unattended=yes
    else
        unattended=no
    fi
fi

# 3. Check time
if [ -n "${UPDATEO_CHECK_TIME:-}" ]; then
    check_time=$UPDATEO_CHECK_TIME
else
    if [ "$ASSUME_YES" = 1 ]; then
        check_time=09:30
    else
        while :; do
            printf 'Daily update check time [09:30]: '
            IFS= read -r check_time || check_time=
            [ -n "$check_time" ] || check_time=09:30
            valid_time "$check_time" && break
            say "Use HH:MM (24-hour), for example 09:30."
        done
    fi
fi
if ! valid_time "$check_time"; then
    say "Invalid check time '$check_time'; expected HH:MM."
    exit 1
fi

# 4. Login check
if [ -n "${UPDATEO_AUTOSTART:-}" ]; then
    case "$UPDATEO_AUTOSTART" in
        y|Y|yes|YES) autostart=yes ;;
        *) autostart=no ;;
    esac
else
    if ask_yn "Run an update check when you log in?" n; then
        autostart=yes
    else
        autostart=no
    fi
fi

say
say "Building release binary…"
cargo build --release

say "Installing binary to $BINDIR/updateo…"
mkdir -p "$BINDIR" 2>/dev/null || true
if [ -w "$BINDIR" ]; then
    install -Dm755 target/release/updateo "$BINDIR/updateo"
else
    $SUDO install -Dm755 target/release/updateo "$BINDIR/updateo"
fi
UPDATEO_BIN=$BINDIR/updateo

say "Writing $CONFIG_DIR/config.json…"
"$UPDATEO_BIN" configure \
    --notify "$notify" \
    --unattended "$unattended" \
    --check-time "$check_time" \
    --autostart "$autostart"

say "Installing daily timer (systemd-user)…"
"$UPDATEO_BIN" install-timer

if [ "$autostart" = yes ]; then
    "$UPDATEO_BIN" install-autostart
else
    "$UPDATEO_BIN" uninstall-autostart >/dev/null 2>&1 || true
fi

if [ "$unattended" = yes ]; then
    say
    say "NOTE: unattended updates run without a terminal. Configure passwordless"
    say "sudo for pacman (and yay if you use AUR), otherwise the timer will fail"
    say "when it needs a password. Example /etc/sudoers.d/updateo:"
    say "  ${USER:-your-user} ALL=(root) NOPASSWD: /usr/bin/pacman"
fi
if [ "$notify" = yes ] && ! command -v notify-send >/dev/null 2>&1; then
    say "WARNING: notify-send is missing; install libnotify for notifications."
fi

say
say "Done. Run 'updateo' for the TUI, or 'updateo --help' for commands."
say "Configuration: $CONFIG_DIR/config.json"
