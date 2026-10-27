//! Veri kaynakları: pacman (repo), AUR (yay üzerinden), Flatpak.
//! Her kaynak tek bir bloklamayan-yakalama çağrısı yapar; çıktı belgelenmiş
//! satır formatlarına göre parse edilir.

use std::process::Command;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Source {
    Pacman,
    Aur,
    Flatpak,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Source::Pacman => "pacman",
            Source::Aur => "AUR",
            Source::Flatpak => "Flatpak",
        }
    }
    pub fn order(self) -> u8 {
        match self {
            Source::Pacman => 0,
            Source::Aur => 1,
            Source::Flatpak => 2,
        }
    }
    pub const ALL: [Source; 3] = [Source::Pacman, Source::Aur, Source::Flatpak];
}

pub struct Update {
    pub source: Source,
    /// pacman/yay paket adı ya da flatpak app-id / runtime ref'i
    pub name: String,
    /// insan okunur ad (flatpak uygulamaları için)
    pub display: String,
    pub from: String,
    pub to: String,
    pub selected: bool,
}

pub type CheckResult = Result<Vec<Update>, String>;

/// Komutu çalıştır, stdout döndür; başarısızlıkta stderr'i hata mesajı yap.
fn capture(cmd: &mut Command) -> Result<String, String> {
    let out = cmd
        .output()
        .map_err(|e| format!("'{:?}' çalıştırılamadı: {e}", cmd.get_program()))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let err = err.trim();
        if !err.is_empty() {
            return Err(err.lines().last().unwrap_or(err).to_string());
        }
        // "güncelleme yok" konvansiyonları (stderr boş):
        //   pacman -Qu / yay -Qua -> exit 1, checkupdates -> exit 2
        if matches!(out.status.code(), Some(1) | Some(2)) {
            return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
        }
        return Err(format!("exit {}", out.status));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// `pacman -Qu` / `checkupdates` / `yay -Qua` satırları: `pkg eski -> yeni`
fn parse_arrow_lines(text: &str, source: Source) -> Vec<Update> {
    let mut v = Vec::new();
    for line in text.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 4 || parts[2] != "->" {
            continue;
        }
        let name = parts[0].trim_start_matches("aur/").to_string();
        v.push(Update {
            source,
            display: name.clone(),
            name,
            from: parts[1].to_string(),
            to: parts[3].to_string(),
            selected: true, // updater aracı: varsayılan seçili, kullanıcı düşürür
        });
    }
    v
}

/// `flatpak remote-ls --updates --columns=app,name,version` satırları
/// (sekme ayrımlı; runtimes için app sütunu `ref/branch` taşır).
fn parse_flatpak_lines(text: &str) -> Vec<Update> {
    let mut v = Vec::new();
    for line in text.lines() {
        let cols: Vec<&str> = line.split('\t').map(str::trim).collect();
        let app = cols.first().copied().unwrap_or("");
        if app.is_empty() {
            continue;
        }
        let name: &str = if cols.len() > 1 && !cols[1].is_empty() {
            cols[1]
        } else {
            app
        };
        let version = cols.get(2).copied().unwrap_or("").to_string();
        v.push(Update {
            source: Source::Flatpak,
            display: name.to_string(),
            name: app.to_string(),
            from: String::new(),
            to: version,
            selected: true,
        });
    }
    v
}

pub fn check_pacman() -> CheckResult {
    // pacman-contrib'un checkupdates'ı öncelikli (kendi geçici db'sini tazeler);
    // yoksa önbellekteki db ile `pacman -Qu`ya düşer.
    if let Ok(out) = capture(&mut Command::new("checkupdates").arg("--nocolor")) {
        return Ok(parse_arrow_lines(&out, Source::Pacman));
    }
    let out = capture(&mut Command::new("pacman").args(["-Qu"]))?;
    Ok(parse_arrow_lines(&out, Source::Pacman))
}

pub fn check_aur() -> CheckResult {
    let out = capture(&mut Command::new("yay").args(["-Qua"]))?;
    Ok(parse_arrow_lines(&out, Source::Aur))
}

pub fn check_flatpak() -> CheckResult {
    let out = capture(&mut Command::new("flatpak").args([
        "remote-ls",
        "--updates",
        "--columns=app,name,version",
    ]))?;
    Ok(parse_flatpak_lines(&out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arrow_lines() {
        let v = parse_arrow_lines(
            "linux 6.17.4.cachyos-1 -> 6.17.5.cachyos-1\n\
             aur/tribler 2.2.2-1 -> 2.3.0-1\n\
             gk 1.2 -> 1.3\n\
             junk line without arrow\n",
            Source::Pacman,
        );
        assert_eq!(v.len(), 3);
        assert_eq!(v[0].name, "linux");
        assert_eq!(v[1].name, "tribler"); // aur/ öneki temizlenir
        assert_eq!(v[1].to, "2.3.0-1");
        assert!(v[0].selected);
    }

    #[test]
    fn flatpak_lines() {
        let v = parse_flatpak_lines(
            "com.microsoft.Edge\tMicrosoft Edge\t156.0.4314.52-1\n\
             org.gnome.Platform/x86_64/51\tGNOME Application Platform version 52\t\n",
        );
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].name, "com.microsoft.Edge");
        assert_eq!(v[0].display, "Microsoft Edge");
        assert_eq!(v[0].to, "156.0.4314.52-1");
        assert_eq!(v[1].name, "org.gnome.Platform/x86_64/51"); // runtime ref'i
    }
}
