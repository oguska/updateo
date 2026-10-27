//! Oturum açılışında arka plan kontrolü: XDG autostart `.desktop` dosyası.
//! TUI açmak yerine `check --notify` çalışır; bildirim ayarı config'ten gelir.

use crate::config;
use crate::i18n::tr;
use std::fs;
use std::io;
use std::path::PathBuf;

fn path() -> io::Result<PathBuf> {
    Ok(config::dir()?
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join("autostart")
        .join("updateo.desktop"))
}

fn desktop_text(exe: &str) -> String {
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=updateo\n\
         Comment=Check for Arch/AUR/Flatpak updates at login\n\
         Exec=\"{exe}\" check --notify\n\
         Terminal=false\n\
         NoDisplay=true\n\
         X-GNOME-Autostart-enabled=true\n"
    )
}

pub fn install() -> io::Result<()> {
    let path = path()?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let exe = std::env::current_exe()?
        .canonicalize()
        .unwrap_or_else(|_| std::env::current_exe().unwrap_or_default());
    fs::write(&path, desktop_text(&exe.display().to_string()))?;
    println!(
        "{}",
        tr(
            &format!("login check installed: {}", path.display()),
            &format!("açılış kontrolü kuruldu: {}", path.display())
        )
    );
    Ok(())
}

pub fn uninstall() -> io::Result<()> {
    let path = path()?;
    match fs::remove_file(&path) {
        Ok(()) => println!(
            "{}",
            tr(
                &format!("login check removed: {}", path.display()),
                &format!("açılış kontrolü kaldırıldı: {}", path.display())
            )
        ),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn autostart_komutu_bildirim_kontrolu_calistirir() {
        let text = desktop_text("/usr/bin/updateo");
        assert!(text.contains("Exec=\"/usr/bin/updateo\" check --notify"));
        assert!(text.contains("X-GNOME-Autostart-enabled=true"));
        assert!(text.contains("NoDisplay=true"));
    }
}
