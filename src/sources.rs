//! Veri kaynakları: pacman (repo), AUR (yay üzerinden), Flatpak.
//! Her kaynak tek bir bloklamayan-yakalama çağrısı yapar; çıktı belgelenmiş
//! satır formatlarına göre parse edilir.

use std::collections::BTreeMap;
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
    /// Flatpak: runtime/uzantı (uygulama değil).
    pub runtime: bool,
    pub selected: bool,
}

pub fn check(src: Source) -> CheckResult {
    match src {
        Source::Pacman => check_pacman(),
        Source::Aur => check_aur(),
        Source::Flatpak => check_flatpak(),
    }
}

pub type CheckResult = Result<Vec<Update>, String>;

/// Yönetici (root) yetkisi gereken komutu kur: varsayılan sarmalayıcı `sudo`;
/// `UPDATEO_SUDO="doas"` gibi bir değerle değiştirilebilir (boşlukla ayrılan
/// ilk parça program, kalanı önek argüman olur — ör. `UPDATEO_SUDO="doas -n"`).
pub fn root_command(program: &str, args: &[&str]) -> Command {
    let wrapper: Vec<String> = std::env::var("UPDATEO_SUDO")
        .unwrap_or_else(|_| "sudo".into())
        .split_whitespace()
        .map(String::from)
        .collect();
    let mut c = Command::new(wrapper.first().map(String::as_str).unwrap_or("sudo"));
    c.args(&wrapper[1..]);
    c.arg(program);
    c.args(args);
    c
}

/// Komutu çalıştır, stdout döndür; başarısızlıkta stderr'i hata mesajı yap.
fn capture(mut cmd: Command) -> Result<String, String> {
    let out = cmd.output().map_err(|e| {
        crate::i18n::tr(
            &format!("could not run '{:?}': {e}", cmd.get_program()),
            &format!("'{:?}' çalıştırılamadı: {e}", cmd.get_program()),
        )
    })?;
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
            runtime: false,
            selected: true, // updater aracı: varsayılan seçili, kullanıcı düşürür
        });
    }
    v
}

/// `flatpak remote-ls --updates --columns=app,name,version` satırları.
/// Runtime/uzantıların app sütunu `ad/mimari/branch` taşır; kurulu sürüm
/// eşleşmesi ilk bölüm (ad) üzerinden yapılır.
fn parse_flatpak_lines(text: &str, installed: &BTreeMap<String, String>) -> Vec<Update> {
    let mut v = Vec::new();
    for line in text.lines() {
        let cols: Vec<&str> = line.split('\t').map(str::trim).collect();
        let app = cols.first().copied().unwrap_or("");
        if app.is_empty() {
            continue;
        }
        let runtime = app.contains('/');
        let key = app.split('/').next().unwrap_or(app);
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
            from: installed.get(key).cloned().unwrap_or_default(),
            to: version,
            runtime,
            selected: true,
        });
    }
    v
}

pub fn check_pacman() -> CheckResult {
    // pacman-contrib'un checkupdates'ı öncelikli (kendi geçici db'sini tazeler);
    // yoksa önbellekteki db ile `pacman -Qu`ya düşer.
    if let Ok(out) = capture(cmd("checkupdates", &["--nocolor"])) {
        return Ok(parse_arrow_lines(&out, Source::Pacman));
    }
    let out = capture(cmd("pacman", &["-Qu"]))?;
    Ok(parse_arrow_lines(&out, Source::Pacman))
}

pub fn check_aur() -> CheckResult {
    let out = capture(cmd("yay", &["-Qua"]))?;
    Ok(parse_arrow_lines(&out, Source::Aur))
}

/// Sahipli `Command` kurucusu (builder zinciri `&mut Command` döndürür).
fn cmd(program: &str, args: &[&str]) -> Command {
    let mut c = Command::new(program);
    c.args(args);
    c
}

/// Kurulu flatpak sürümleri: `flatpak list --columns=app,version`.
/// Runtime satırlarında app sütunu çıplak addır (mimari/branch taşımaz).
fn flatpak_installed() -> BTreeMap<String, String> {
    let mut m = BTreeMap::new();
    if let Ok(out) = capture(cmd("flatpak", &["list", "--columns=app,version"])) {
        for line in out.lines() {
            let cols: Vec<&str> = line.split('\t').map(str::trim).collect();
            let (Some(app), Some(ver)) = (cols.first(), cols.get(1)) else {
                continue;
            };
            if !app.is_empty() {
                m.entry(app.to_string()).or_insert_with(|| ver.to_string());
            }
        }
    }
    m
}

pub fn check_flatpak() -> CheckResult {
    let installed = flatpak_installed();
    let out = capture(cmd(
        "flatpak",
        &["remote-ls", "--updates", "--columns=app,name,version"],
    ))?;
    Ok(parse_flatpak_lines(&out, &installed))
}

/// Sıralama ölçütü: kaynak → düz uygulamalar önce, runtime/uzantılar sonra → ad.
pub fn sort_updates(v: &mut [Update]) {
    v.sort_by(|a, b| {
        a.source
            .order()
            .cmp(&b.source.order())
            .then(a.runtime.cmp(&b.runtime))
            .then(a.name.cmp(&b.name))
    });
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
        let installed = BTreeMap::from([
            ("com.microsoft.Edge".into(), "156.0.4314.50-1".into()),
            ("org.gnome.Platform".into(), "51.6".into()),
        ]);
        let v = parse_flatpak_lines(
            "com.microsoft.Edge\tMicrosoft Edge\t156.0.4314.52-1\n\
             org.gnome.Platform/x86_64/51\tGNOME Application Platform version 52\t52.4\n",
            &installed,
        );
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].name, "com.microsoft.Edge");
        assert_eq!(v[0].display, "Microsoft Edge");
        assert_eq!(v[0].from, "156.0.4314.50-1");
        assert!(!v[0].runtime);
        assert_eq!(v[1].name, "org.gnome.Platform/x86_64/51");
        assert_eq!(v[1].to, "52.4");
        assert!(v[1].runtime);
        // kurulu eşleşmesi ref'in ilk bölümüyle: branch (x86_64/51) değil
        assert_eq!(v[1].from, "51.6");
    }

    #[test]
    fn runtime_sirada_sonra() {
        let installed = BTreeMap::new();
        let mut v = parse_flatpak_lines(
            "org.gnome.Platform/x86_64/51\tPlatform\t52.4\n\
             com.microsoft.Edge\tEdge\t1.2\n\
             org.freedesktop.Platform.GL.default\tGL\t26.3\n",
            &installed,
        );
        sort_updates(&mut v);
        assert_eq!(v[0].name, "com.microsoft.Edge"); // düz uygulama en önce
        assert_eq!(v[1].name, "org.freedesktop.Platform.GL.default");
        assert_eq!(v[2].name, "org.gnome.Platform/x86_64/51");
    }
}
