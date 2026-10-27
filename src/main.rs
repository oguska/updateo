//! updateo — pacman + AUR (yay) + Flatpak güncelleyicisi, ratatui tabanlı TUI.
//!
//! Komutlar:
//!   updateo                       TUI
//!   updateo check [--notify]      başlıksız kontrol (script/cron için), istersen bildirim
//!   updateo install-timer         systemd-user günlük zamanlayıcısını kur
//!   updateo uninstall-timer       zamanlayıcıyı kaldır

mod app;
mod autostart;
mod config;
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
/// Sürüm tek kaynaktan: Cargo.toml `package.version`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() -> io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("check") => cmd_check(args.iter().any(|a| a == "--notify")),
        Some("update") => cmd_update(
            args.iter().any(|a| a == "--yes"),
            args.iter().any(|a| a == "--notify"),
        ),
        Some("configure") => cmd_configure(&args[1..]),
        Some("install-timer") => timer::install(),
        Some("uninstall-timer") => timer::uninstall(),
        Some("install-autostart") => autostart::install(),
        Some("uninstall-autostart") => autostart::uninstall(),
        Some("-V" | "--version") => {
            println!("updateo {VERSION}");
            Ok(())
        }
        Some("-h" | "--help") => {
            println!(
                "{}",
                tr(
                    &format!(
                        "updateo {VERSION}: pacman + AUR + Flatpak updater\n  (no args)        TUI\n  check [--notify] headless check; --notify shows an actionable desktop notification\n  update --yes [--notify]   apply all updates without prompts (automation)\n  configure [--notify yes|no] [--unattended yes|no] [--check-time HH:MM] [--autostart yes|no]\n  install-timer / uninstall-timer   systemd-user daily timer (config time)\n  install-autostart / uninstall-autostart   check at login\n  -V / --version   print version"
                    ),
                    &format!(
                        "updateo {VERSION}: pacman + AUR + Flatpak güncelleyicisi\n  (argsız)          TUI\n  check [--notify]  başlıksız kontrol; --notify eylemli masaüstü bildirimi gösterir\n  update --yes [--notify]   tüm güncellemeleri sormadan uygula (otomasyon)\n  configure [--notify evet|hayır] [--unattended evet|hayır] [--check-time SS:DD] [--autostart evet|hayır]\n  install-timer / uninstall-timer  systemd-user günlük zamanlayıcı (config saati)\n  install-autostart / uninstall-autostart  açılışta kontrol\n  -V / --version   sürümü yazdır"
                    )
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
        // Askıya alınmış TUI'den dönüşte terminal temizlenmiş olabilir;
        // ratatui önbelleğini de sıfırla ki tüm çerçeve yeniden çizilsin.
        if app.take_redraw() {
            // `resize` aynı alanla da çağrılabilir: ekranı temizleyip ratatui
            // önbelleğini sıfırlar. `Terminal::clear` imleç konumu sorgular ve
            // yanıt vermeyen terminallerde kilitlenebilir.
            let area: ratatui::layout::Rect = terminal.size()?.into();
            terminal.resize(area)?;
        }
        terminal.draw(|f| app.draw(f))?;
        app.handle_events()?;
    }

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

/// Config'i bayraklarla güncelle; bootstrap bunu kullanır. Bayraksız çağrı
/// mevcut yapılandırmayı JSON olarak yazdırır.
fn cmd_configure(args: &[String]) -> io::Result<()> {
    if args.is_empty() {
        let text = serde_json::to_string_pretty(&config::load())
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        println!("{text}");
        return Ok(());
    }
    let mut cfg = config::load();
    let mut i = 0;
    while i < args.len() {
        let flag = args[i].as_str();
        let value = args.get(i + 1).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                tr(
                    &format!("{flag} needs a value"),
                    &format!("{flag} bir değer gerektirir"),
                ),
            )
        })?;
        match flag {
            "--notify" => cfg.notify = parse_yes_no(value, flag)?,
            "--unattended" => cfg.unattended = parse_yes_no(value, flag)?,
            "--autostart" => cfg.autostart = parse_yes_no(value, flag)?,
            "--check-time" => {
                cfg.check_time = config::normalize_time(value).ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        tr(
                            &format!("invalid time '{value}'; expected HH:MM"),
                            &format!("geçersiz saat '{value}'; SS:DD bekleniyor"),
                        ),
                    )
                })?
            }
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    tr(
                        &format!("unknown option {flag}"),
                        &format!("bilinmeyen seçenek {flag}"),
                    ),
                ));
            }
        }
        i += 2;
    }
    let path = config::save(&cfg)?;
    println!(
        "{}",
        tr(
            &format!(
                "configuration written: {} (notify={}, unattended={}, check_time={}, autostart={})",
                path.display(),
                cfg.notify,
                cfg.unattended,
                cfg.check_time,
                cfg.autostart
            ),
            &format!(
                "yapılandırma yazıldı: {} (bildirim={}, onaysız={}, saat={}, açılış={})",
                path.display(),
                cfg.notify,
                cfg.unattended,
                cfg.check_time,
                cfg.autostart
            )
        )
    );
    Ok(())
}

fn parse_yes_no(value: &str, flag: &str) -> io::Result<bool> {
    match value.to_ascii_lowercase().as_str() {
        "yes" | "y" | "true" | "1" | "evet" | "e" => Ok(true),
        "no" | "n" | "false" | "0" | "hayır" | "hayir" | "h" => Ok(false),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            tr(
                &format!("{flag} expects yes/no, got '{value}'"),
                &format!("{flag} evet/hayır bekler, '{value}' verildi"),
            ),
        )),
    }
}

/// TUI'siz kontrol: kaynak bazında sayı + liste yazdırır.
/// `--notify` ve config'te bildirim açıksa eylemli masaüstü bildirimi gönderir.
fn cmd_check(notify: bool) -> io::Result<()> {
    let config = config::load();
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
    } else if notify && config.notify {
        let body = per_source
            .iter()
            .map(|(s, n)| format!("{}: {n}", s.label()))
            .collect::<Vec<_>>()
            .join(", ");
        notify_action(
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
    }
    Ok(())
}

/// Onaysız toplu güncelleme. Systemd timer'ı `update --yes --notify` çalıştırır;
/// sudo'nun parola sormaması gerekir (NOPASSWD/polkit).
fn cmd_update(yes: bool, notify: bool) -> io::Result<()> {
    if !yes {
        eprintln!(
            "{}",
            tr(
                "refusing to update without --yes; use the TUI for interactive updates",
                "--yes olmadan güncelleme yapılmaz; etkileşimli güncelleme için TUI'yi kullanın"
            )
        );
        return Ok(());
    }
    let config = config::load();
    let mut total = 0usize;
    let mut applied = 0usize;
    let mut failed: Vec<String> = Vec::new();
    let mut counts: Vec<(Source, usize)> = Vec::new();

    for src in Source::ALL {
        let list = match sources::check(src) {
            Ok(list) => list,
            Err(e) => {
                eprintln!(
                    "{}",
                    tr(
                        &format!("{} check failed: {e}", src.label()),
                        &format!("{} kontrolü başarısız: {e}", src.label()),
                    )
                );
                failed.push(src.label().to_string());
                continue;
            }
        };
        if list.is_empty() {
            continue;
        }
        let names: Vec<String> = list.iter().map(|u| u.name.clone()).collect();
        total += names.len();
        counts.push((src, names.len()));
        println!(
            "{}",
            tr(
                &format!("{}: updating {} package(s)…", src.label(), names.len()),
                &format!("{}: {} paket güncelleniyor…", src.label(), names.len()),
            )
        );
        let mut cmd = sources::unattended_update_command(src, &names);
        cmd.stdin(Stdio::null());
        let success = cmd.status().is_ok_and(|st| st.success());
        if success {
            applied += names.len();
        } else {
            failed.push(src.label().to_string());
        }
        history::record(src.label(), if success { "ok" } else { "failed" }, &names);
    }

    if total == 0 && failed.is_empty() {
        println!("{}", tr("system is up to date", "sistem güncel"));
        return Ok(());
    }
    if notify && config.notify {
        let body = counts
            .iter()
            .map(|(s, n)| format!("{}: {n}", s.label()))
            .collect::<Vec<_>>()
            .join(", ");
        let (title, text, label) = if failed.is_empty() {
            (
                tr(
                    "updateo: updates installed",
                    "updateo: güncellemeler kuruldu",
                ),
                tr(
                    &format!("{applied} package(s) updated ({body})."),
                    &format!("{applied} paket güncellendi ({body})."),
                ),
                tr("Open updateo", "updateo'yu aç"),
            )
        } else {
            (
                tr("updateo: update failed", "updateo: güncelleme başarısız"),
                tr(
                    &format!(
                        "{applied}/{total} package(s) updated; failed: {}",
                        failed.join(", ")
                    ),
                    &format!(
                        "{total} paketten {applied} tanesi güncellendi; başarısız: {}",
                        failed.join(", ")
                    ),
                ),
                tr("Open updateo", "updateo'yu aç"),
            )
        };
        notify_action(&title, &text, &label);
    }
    if failed.is_empty() {
        Ok(())
    } else {
        Err(io::Error::other(tr(
            &format!("update failed for: {}", failed.join(", ")),
            &format!("güncelleme başarısız: {}", failed.join(", ")),
        )))
    }
}

/// Bildirimi göster; kullanıcı eylem düğmesine basarsa TUI'yi aç.
fn notify_action(title: &str, body: &str, action_label: &str) {
    if notify_send(title, body, action_label).as_deref() == Some(NOTIFY_ACTION)
        && let Err(e) = launch_tui()
    {
        eprintln!(
            "{}",
            tr(
                &format!("could not open a terminal for updateo: {e}; run 'updateo' manually"),
                &format!("updateo için uçbirim açılamadı: {e}; 'updateo' komutunu elle çalıştırın"),
            )
        );
    }
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
