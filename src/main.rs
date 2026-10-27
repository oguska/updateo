//! updateo — pacman + AUR (yay) + Flatpak güncelleyicisi, ratatui tabanlı TUI.
//!
//! Komutlar:
//!   updateo                       TUI
//!   updateo check [--notify]      başlıksız kontrol (script/cron için), istersen bildirim
//!   updateo install-timer         systemd-user günlük zamanlayıcısını kur
//!   updateo uninstall-timer       zamanlayıcıyı kaldır

mod app;
mod history;
mod i18n;
mod runner;
mod sources;
mod timer;

use app::App;
use crossterm::{
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use i18n::tr;
use ratatui::{Terminal, backend::CrosstermBackend};
use sources::Source;
use std::ffi::{OsStr, OsString};
use std::io;
use std::path::Path;
use std::process::{Command, Stdio};

/// Bildirimdeki eylem anahtarı; `notify-send` seçilen eylemi stdout'a yazar.
const NOTIFY_ACTION: &str = "update";
/// Bildirim eylem düğmesi ne kadar görünür kalsın (ms).
const NOTIFY_TIMEOUT_MS: &str = "45000";

fn main() -> io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("check") => cmd_check(args.iter().any(|a| a == "--notify")),
        Some("install-timer") => timer::install(),
        Some("uninstall-timer") => timer::uninstall(),
        Some("-h" | "--help") => {
            println!(
                "{}",
                tr(
                    "updateo: pacman + AUR + Flatpak updater\n  (no args)        TUI\n  check [--notify] headless check; --notify shows an actionable desktop notification\n  install-timer / uninstall-timer   systemd-user daily timer",
                    "updateo: pacman + AUR + Flatpak güncelleyicisi\n  (argsız)          TUI\n  check [--notify]  başlıksız kontrol; --notify eylemli masaüstü bildirimi gösterir\n  install-timer / uninstall-timer  systemd-user günlük zamanlayıcısı"
                )
            );
            Ok(())
        }
        _ => run_tui(),
    }
}

/// TUI: alternate ekran + ham mod, panikte geri al.
fn run_tui() -> io::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // panik halinde terminal eski haline dönsün
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        hook(info);
    }));

    let mut app = App::new();
    app.start_refresh();

    while !app.quit {
        terminal.draw(|f| app.draw(f))?;
        app.handle_events()?;
    }

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

/// TUI'siz kontrol: kaynak bazında sayı + liste yazdırır.
/// `--notify` ve güncelleme varsa masaüstü bildirimi gönderir.
fn cmd_check(notify: bool) -> io::Result<()> {
    let mut total = 0usize;
    let mut per_source: Vec<(Source, usize)> = Vec::new();
    for src in Source::ALL {
        match sources::check(src) {
            Ok(list) => {
                total += list.len();
                per_source.push((src, list.len()));
                if !list.is_empty() {
                    println!(
                        "{}",
                        tr(
                            &format!("{}: {} updateable", src.label(), list.len()),
                            &format!("{}: {} güncellenebilir", src.label(), list.len()),
                        )
                    );
                    for u in &list {
                        let ver = if u.from.is_empty() {
                            &u.to
                        } else {
                            &format!("{} -> {}", u.from, u.to)
                        };
                        println!("  {} {ver}", u.display);
                    }
                }
            }
            Err(e) => {
                per_source.push((src, 0));
                eprintln!(
                    "{}",
                    tr(
                        &format!("{} check failed: {e}", src.label()),
                        &format!("{} kontrolü başarısız: {e}", src.label()),
                    )
                );
            }
        }
    }
    if total == 0 {
        println!("{}", tr("system is up to date", "sistem güncel"));
    } else if notify {
        let body = per_source
            .iter()
            .map(|(s, n)| format!("{}: {n}", s.label()))
            .collect::<Vec<_>>()
            .join(", ");
        let action = notify_send(
            &tr("updateo: updates available", "updateo: güncelleme var"),
            &tr(
                &format!(
                    "{total} updateable package(s) ({body})\nOpen updateo to review and apply."
                ),
                &format!(
                    "{total} güncellenebilir paket ({body})\nİncelemek ve uygulamak için updateo'yu aç."
                ),
            ),
            &tr("Update now", "Şimdi güncelle"),
        );
        if action.as_deref() == Some(NOTIFY_ACTION)
            && let Err(e) = launch_tui()
        {
            eprintln!(
                "{}",
                tr(
                    &format!("could not open a terminal for updateo: {e}; run 'updateo' manually"),
                    &format!(
                        "updateo için uçbirim açılamadı: {e}; 'updateo' komutunu elle çalıştırın"
                    ),
                )
            );
        }
    }
    Ok(())
}

/// Masaüstü bildirimi: eylem düğmesi seçilirse eylem anahtarını döndürür.
/// notify-send yoksa/başarısızsa metni journal'a düşürür.
fn notify_send(title: &str, body: &str, action_label: &str) -> Option<String> {
    let output = notification_command(title, body, action_label).output();
    match output {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .map(str::to_string),
        Ok(out) => {
            eprintln!("{title}: {body}");
            let err = String::from_utf8_lossy(&out.stderr);
            if !err.trim().is_empty() {
                eprintln!("{}", err.trim());
            }
            None
        }
        Err(_) => {
            eprintln!("{title}: {body}");
            None
        }
    }
}

/// Bildirim komutu: eylem düğmesi tıklanınca notify-send anahtarı stdout'a yazar.
fn notification_command(title: &str, body: &str, action_label: &str) -> Command {
    let mut cmd = Command::new("notify-send");
    cmd.args([
        "-a",
        "updateo",
        "-i",
        "system-software-update",
        "-t",
        NOTIFY_TIMEOUT_MS,
        "-w",
    ])
    .arg(format!("--action={NOTIFY_ACTION}={action_label}"))
    .args([title, body]);
    cmd
}

/// Bildirim eyleminden TUI'yi masaüstü uçbiriminde aç. `UPDATEO_TERMINAL`
/// ile belirli bir uçbirim yolu seçilebilir; yoksa yaygın olanlar denenir.
fn launch_tui() -> io::Result<()> {
    let exe = std::env::current_exe()?;
    if let Some(term) = std::env::var_os("UPDATEO_TERMINAL").filter(|v| !v.is_empty()) {
        return spawn_in_terminal(&term, &exe);
    }
    let mut last_err = None;
    for term in [
        "xdg-terminal-exec",
        "konsole",
        "alacritty",
        "kitty",
        "foot",
        "wezterm",
        "gnome-terminal",
        "xfce4-terminal",
        "xterm",
    ] {
        match spawn_in_terminal(OsStr::new(term), &exe) {
            Ok(()) => return Ok(()),
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.unwrap_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            tr("no terminal emulator found", "uçbirim bulunamadı"),
        )
    }))
}

fn spawn_in_terminal(terminal: &OsStr, exe: &Path) -> io::Result<()> {
    let mut cmd = Command::new(terminal);
    cmd.args(terminal_args(terminal, exe))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
}

/// Uçbirimlerin "şu komutu çalıştır" sözdizimi farklı; ada göre argüman kur.
fn terminal_args(terminal: &OsStr, exe: &Path) -> Vec<OsString> {
    let name = Path::new(terminal)
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or_default();
    match name {
        "xdg-terminal-exec" => vec![exe.as_os_str().to_owned()],
        "wezterm" => vec!["start".into(), "--".into(), exe.as_os_str().to_owned()],
        "gnome-terminal" => vec!["--".into(), exe.as_os_str().to_owned()],
        "xfce4-terminal" => vec!["-x".into(), exe.as_os_str().to_owned()],
        _ => vec!["-e".into(), exe.as_os_str().to_owned()],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bildirim_eylem_duegmesi_eklenir() {
        let cmd = notification_command("başlık", "gövde", "Şimdi güncelle");
        let args: Vec<String> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(args.iter().any(|a| a == "--action=update=Şimdi güncelle"));
        assert!(args.contains(&"-w".to_string()));
        assert_eq!(&args[args.len() - 2..], ["başlık", "gövde"]);
    }

    #[test]
    fn ucbirim_argumanlari_ada_gore_kurulur() {
        let exe = Path::new("/usr/bin/updateo");
        let args = |term: &str| -> Vec<String> {
            terminal_args(OsStr::new(term), exe)
                .iter()
                .map(|a| a.to_string_lossy().into_owned())
                .collect()
        };
        assert_eq!(args("konsole"), ["-e", "/usr/bin/updateo"]);
        assert_eq!(args("xdg-terminal-exec"), ["/usr/bin/updateo"]);
        assert_eq!(args("wezterm"), ["start", "--", "/usr/bin/updateo"]);
        assert_eq!(args("gnome-terminal"), ["--", "/usr/bin/updateo"]);
    }
}
