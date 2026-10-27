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
use std::io;
use std::process::Command;

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
                    "updateo: pacman + AUR + Flatpak updater\n  (no args)        TUI\n  check [--notify] headless check, optional desktop notification\n  install-timer / uninstall-timer   systemd-user daily timer",
                    "updateo: pacman + AUR + Flatpak güncelleyicisi\n  (argsız)          TUI\n  check [--notify]  başlıksız kontrol, istenirse masaüstü bildirimi\n  install-timer / uninstall-timer  systemd-user günlük zamanlayıcısı"
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
        notify_send(
            &tr("updateo: updates available", "updateo: güncelleme var"),
            &tr(
                &format!("{total} updateable package(s) ({body})"),
                &format!("{total} güncellenebilir paket ({body})"),
            ),
        );
    }
    Ok(())
}

/// notify-send dene; yoksa standart hataya düş (systemd journal yakalar).
fn notify_send(title: &str, body: &str) {
    match Command::new("notify-send")
        .args(["-a", "updateo", title, body])
        .status()
    {
        Ok(st) if st.success() => {}
        _ => eprintln!("{title}: {body}"),
    }
}
