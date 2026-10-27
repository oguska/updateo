//! Veri kaynakları: pacman (repo), AUR (yay üzerinden), Flatpak.
//! Her kaynak tek bir bloklamayan-yakalama çağrısı yapar; çıktı belgelenmiş
//! satır formatlarına göre parse edilir.

use std::collections::BTreeMap;
use std::process::Command;

use crate::i18n::tr;

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
    let wrapper = sudo_wrapper();
    with_wrapper(&wrapper, program, args)
}

/// Yetki sarmalayıcısı: varsayılan `sudo`, `UPDATEO_SUDO` ile değiştirilir.
pub fn sudo_wrapper() -> String {
    std::env::var("UPDATEO_SUDO")
        .unwrap_or_else(|_| "sudo".into())
        .trim()
        .to_string()
}

/// `UPDATEO_SUDO` sözdizimini (ilk parça program, kalanı önek argüman)
/// uygulayan saf çekirdek; env değişkeni okumaz, test edilebilir.
fn with_wrapper(wrapper: &str, program: &str, args: &[&str]) -> Command {
    let parts: Vec<&str> = wrapper.split_whitespace().collect();
    let mut c = Command::new(parts.first().copied().unwrap_or("sudo"));
    if let Some(rest) = parts.get(1..) {
        c.args(rest);
    }
    c.arg(program);
    c.args(args);
    c
}

/// Tek paket kurma komutu: onay soruları bayraklarla kapatılır (pacman/yay
/// `--noconfirm`, flatpak `-y`); yalnız sudo parolası sorulabilir. pacman kök
/// sarmalayıcıdan geçer; yay ve flatpak yetkiyi kendi yönetir.
pub fn install_command(src: Source, names: &[String]) -> Command {
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    match src {
        Source::Pacman => root_command(
            "pacman",
            &[&["-S", "--noconfirm"], names.as_slice()].concat(),
        ),
        Source::Aur => cmd("yay", &[&["-S", "--noconfirm"], names.as_slice()].concat()),
        Source::Flatpak => cmd("flatpak", &[&["install", "-y"], names.as_slice()].concat()),
    }
}

/// Kaldırma komutu: AUR paketleri de yerel pacman veritabanında olduğu için
/// ikisi de `pacman -Rns` (bağımlılık artıkları dahil); flatpak kendi
/// deposundan kaldırır ve yetkiyi kendisi yönetir.
pub fn remove_command(src: Source, names: &[String]) -> Command {
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    match src {
        Source::Pacman | Source::Aur => {
            root_command("pacman", &[&["-Rns"], names.as_slice()].concat())
        }
        Source::Flatpak => cmd("flatpak", &[&["uninstall"], names.as_slice()].concat()),
    }
}

/// Boşlukla ayrılmış paket adlarını ayrıştır. Seçenek enjeksiyonunu engellemek
/// için `-` ile başlayan ad reddedilir; boş girdi de hatadır.
pub fn split_names(raw: &str) -> Result<Vec<String>, String> {
    let mut names: Vec<String> = Vec::new();
    for tok in raw.split_whitespace() {
        if !names.iter().any(|n| n == tok) {
            names.push(tok.to_string());
        }
    }
    if names.is_empty() {
        return Err(tr("empty package name", "boş paket adı"));
    }
    if let Some(bad) = names.iter().find(|n| n.starts_with('-')) {
        return Err(tr(
            &format!("invalid name '{bad}': package names may not start with '-'"),
            &format!("geçersiz ad '{bad}': paket adı '-' ile başlayamaz"),
        ));
    }
    Ok(names)
}

/// Kaldırma öncesi toplanan tek hedef bilgisi: kurulu sürüm, bu paketi
/// kullananlar ve kaldırmanın engellenme gerekçesi.
#[derive(Debug, Clone)]
pub struct Removal {
    pub name: String,
    pub installed: Option<String>,
    /// pacman/AUR: ters bağımlılar; Flatpak: bu runtime'ı kullanan uygulamalar.
    pub dependents: Vec<String>,
    /// Sorgu sırasında alınan uyarı (ör. `pactree` kurulu değil).
    pub note: Option<String>,
    /// Kaldırma arayüzde engellendi: gerekçesiyle birlikte.
    pub blocked: Option<String>,
    /// Onay penceresinde seçili mi (engelli hedefler hiç seçilmez).
    pub include: bool,
}

/// Kaldırma onayına giden hedef listesi: her ad için sistem sorgusu yapılır.
pub fn removal_plan(src: Source, names: &[String]) -> Vec<Removal> {
    names.iter().map(|n| removal_for(src, n)).collect()
}

fn removal_for(src: Source, name: &str) -> Removal {
    let installed = installed_version(src, name);
    let (mut dependents, note) = match src {
        Source::Pacman | Source::Aur => match reverse_deps(name) {
            Ok(v) => (v, None),
            Err(e) => (Vec::new(), Some(e)),
        },
        Source::Flatpak => (flatpak_runtime_users(name), None),
    };
    dependents.sort();
    dependents.dedup();
    let blocked = if is_protected(name) {
        Some(tr(
            "protected base/system package",
            "korunan taban/sistem paketi",
        ))
    } else if installed.is_none() {
        Some(tr(
            &format!("'{name}' is not installed"),
            &format!("'{name}' kurulu değil"),
        ))
    } else {
        None
    };
    Removal {
        name: name.to_string(),
        installed,
        dependents,
        note,
        include: blocked.is_none(),
        blocked,
    }
}

/// Kurulu sürüm: pacman/AUR yerel veritabanından (`pacman -Q`), Flatpak
/// `flatpak list --columns=ref,version` üzerinden (ad ya da tam ref ile).
pub fn installed_version(src: Source, name: &str) -> Option<String> {
    match src {
        Source::Pacman | Source::Aur => capture(cmd("pacman", &["-Q", name]))
            .ok()
            .and_then(|out| parse_q_version(&out)),
        Source::Flatpak => capture(cmd("flatpak", &["list", "--columns=ref,version"]))
            .ok()
            .and_then(|out| parse_ref_version(&out, name)),
    }
}

/// `pacman -Q sed` → `sed 4.10-2.1`; paket yoksa komut başarısız olur (None).
fn parse_q_version(text: &str) -> Option<String> {
    text.lines()
        .next()?
        .split_whitespace()
        .nth(1)
        .map(str::to_string)
}

/// `ref<TAB>version` satırları: birebir ref yoksa adın ilk bölümü eşleşir.
fn parse_ref_version(text: &str, target: &str) -> Option<String> {
    let mut by_name: Option<String> = None;
    for line in text.lines() {
        let cols: Vec<&str> = line.split('\t').map(str::trim).collect();
        let (Some(rf), Some(ver)) = (cols.first().copied(), cols.get(1).copied()) else {
            continue;
        };
        if rf == target {
            return Some(ver.to_string());
        }
        if rf.split('/').next() == Some(target) && by_name.is_none() {
            by_name = Some(ver.to_string());
        }
    }
    by_name
}

/// Kurma/kaldırma komutunun insan okunur dökümü (giriş satırı ve onay
/// penceresinde gösterilir; çalıştırılan komutla aynı sözdizimi).
pub fn install_preview(src: Source) -> String {
    match src {
        Source::Pacman => format!("{} pacman -S --noconfirm", sudo_wrapper()),
        Source::Aur => "yay -S --noconfirm".into(),
        Source::Flatpak => "flatpak install -y".into(),
    }
}

pub fn remove_preview(src: Source) -> String {
    match src {
        // AUR paketleri de yerel veritabanındadır: kaldırma pacman ile.
        Source::Pacman | Source::Aur => format!("{} pacman -Rns", sudo_wrapper()),
        Source::Flatpak => "flatpak uninstall".into(),
    }
}

/// Kaldırılması sisteme zarar veren paketler: meta küme ve paket yöneticisi
/// kendisi. Kaldırma girişimi bu yüzden arayüzde engellenir.
pub const PROTECTED: &[&str] = &[
    "base",
    "base-devel",
    "filesystem",
    "pacman",
    "libalpm",
    "mkinitcpio",
    "systemd",
];

/// Paket koruma altında mı? (adı birebir eşit; alt paketler değil)
pub fn is_protected(name: &str) -> bool {
    PROTECTED.contains(&name)
}

/// Bu paketi ne çekiyor (ters bağımlılar, tek derinlik): `pactree -r --depth=1`.
pub fn reverse_deps(name: &str) -> Result<Vec<String>, String> {
    let out = capture(cmd("pactree", &["-r", "--depth=1", name]))?;
    Ok(parse_reverse_deps(&out))
}

/// Bu flatpak runtime'ını/uzantısını kullanan uygulamalar:
/// `flatpak list --columns=app,runtime`. Uygulama hedefleri için sonuç boştur.
pub fn flatpak_runtime_users(name: &str) -> Vec<String> {
    let out = match capture(cmd("flatpak", &["list", "--columns=app,runtime"])) {
        Ok(o) => o,
        Err(_) => return Vec::new(),
    };
    parse_runtime_users(&out, name)
}

/// Satır ayrımı: `app<TAB>runtime`; runtime `ad/mimari/branch` biçiminde.
/// Hedefın ilk bölümü (ad) ile eşleşenler, hedefın kendisi hariç döner.
fn parse_runtime_users(text: &str, target: &str) -> Vec<String> {
    let want = target.split('/').next().unwrap_or(target);
    let mut v: Vec<String> = Vec::new();
    for line in text.lines() {
        let cols: Vec<&str> = line.split('\t').map(str::trim).collect();
        let (Some(app), Some(runtime)) = (cols.first().copied(), cols.get(1).copied()) else {
            continue;
        };
        let base = runtime.split('/').next().unwrap_or("");
        if base.is_empty() || base != want || app == want || app.split('/').next() == Some(want) {
            continue;
        }
        if !v.iter().any(|a| a == app) {
            v.push(app.to_string());
        }
    }
    v
}

/// `pactree -r` ağacı: ilk satır sorgu paketi, kalan satırlar `├─pkg` /
/// `└─pkg` / `│  ` çizgileriyle gelir. Çizgi önekleri atılır, sırada kalınır.
fn parse_reverse_deps(text: &str) -> Vec<String> {
    let mut seen: Vec<String> = Vec::new();
    for line in text.lines().skip(1) {
        let Some(start) = line.find(|c: char| c.is_alphanumeric()) else {
            continue;
        };
        let dep = line[start..].split_whitespace().next().unwrap_or("");
        if dep.is_empty() || seen.iter().any(|s| s == dep) {
            continue;
        }
        seen.push(dep.to_string());
    }
    seen
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
    fn adlar_secenek_ile_baslayamaz() {
        // tekrar eden ad tek kalır, boşlık ayracı yok
        assert_eq!(split_names(" nano   sed nano ").unwrap(), ["nano", "sed"]);
        // seçenek gibi görünen ad reddedilir: komut enjeksiyonu olmaz
        let err = split_names("-Rns nope").unwrap_err();
        assert!(err.contains("'-Rns'"), "gerekçede ad geçmeli: {err}");
        assert!(split_names("--noconfirm").is_err());
        assert!(split_names("nano -Rns").is_err());
        // bos girdi ayri bir gerekce
        assert!(split_names("   ").is_err());
    }

    #[test]
    fn kurma_komutlari() {
        // Kurulumlar onay sormaz: Y girdisi gerekmez.
        let hello = ["hello".to_string()];
        let c = install_command(Source::Aur, &hello);
        assert_eq!(c.get_program(), "yay");
        assert_eq!(
            c.get_args().collect::<Vec<_>>(),
            ["-S", "--noconfirm", "hello"]
        );

        let edge = ["com.microsoft.Edge".to_string()];
        let c = install_command(Source::Flatpak, &edge);
        assert_eq!(c.get_program(), "flatpak");
        assert_eq!(
            c.get_args().collect::<Vec<_>>(),
            ["install", "-y", "com.microsoft.Edge"]
        );

        let sed = ["sed".to_string()];
        let c = install_command(Source::Pacman, &sed);
        let args: Vec<std::ffi::OsString> = c.get_args().map(|a| a.to_os_string()).collect();
        assert_eq!(
            c.get_program(),
            std::ffi::OsStr::new(sudo_wrapper().as_str())
        );
        assert_eq!(
            args[args.len() - 4..],
            ["pacman", "-S", "--noconfirm", "sed"].map(std::ffi::OsString::from)
        );

        // birden çok ad tek komutta: pacman kök sarmalayıcıdan geçer
        let pkgs = ["sed".to_string(), "nano".to_string()];
        let c = remove_command(Source::Pacman, &pkgs);
        let args: Vec<std::ffi::OsString> = c.get_args().map(|a| a.to_os_string()).collect();
        assert_eq!(
            c.get_program(),
            std::ffi::OsStr::new(sudo_wrapper().as_str())
        );
        assert_eq!(
            args[args.len() - 4..],
            ["pacman", "-Rns", "sed", "nano"].map(std::ffi::OsString::from)
        );
        let c = remove_command(Source::Flatpak, &["org.gnome.Maps".to_string()]);
        assert_eq!(c.get_program(), "flatpak");
        assert_eq!(
            c.get_args().collect::<Vec<_>>(),
            ["uninstall", "org.gnome.Maps"]
        );
    }

    #[test]
    fn ad_ayristirma_guvenli() {
        let v = split_names("  sed   nano\nsed ").unwrap();
        assert_eq!(v, ["sed", "nano"]); // tekrarlar elenir, sıra korunur
        // seçenek enjeksiyonu reddedilir (mesaj dile göre değişir)
        assert!(split_names("--noconfirm").is_err());
        assert!(split_names("-S").is_err());
        assert!(split_names("   ").is_err());
    }

    #[test]
    fn koruma_ve_ref_ayrimi() {
        assert!(is_protected("filesystem"));
        assert!(!is_protected("sed"));
        assert_eq!(
            parse_ref_version("org.gnome.Platform/x86_64/51\t52.4", "org.gnome.Platform")
                .as_deref(),
            Some("52.4")
        );
        assert_eq!(
            parse_ref_version("org.gnome.Platform/x86_64/51\t52.4", "org.kde"),
            None
        );
        assert_eq!(parse_q_version("sed 4.10-2.1").as_deref(), Some("4.10-2.1"));
        let users = parse_runtime_users(
            "org.gnome.Maps\torg.gnome.Platform/x86_64/51\n\
             org.gnome.Platform/x86_64/51\torg.gnome.Platform/x86_64/51\n",
            "org.gnome.Platform",
        );
        assert_eq!(users, ["org.gnome.Maps"]);
    }

    #[test]
    fn sarmalayici_sozdizimi() {
        // sarmalayıcı sözdizimi: doas + önek argüman pacman'e iletilir
        let c = with_wrapper("doas -n", "pacman", &["-S", "hello"]);
        assert_eq!(c.get_program(), "doas");
        assert_eq!(
            c.get_args().collect::<Vec<_>>(),
            ["-n", "pacman", "-S", "hello"]
        );

        // boş sarmalayıcı varsayılana düşer
        let c = with_wrapper("", "pacman", &["-Sy"]);
        assert_eq!(c.get_program(), "sudo");
        assert_eq!(c.get_args().collect::<Vec<_>>(), ["pacman", "-Sy"]);
    }

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
