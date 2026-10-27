//! Güncelleme geçmişi: uygulanan güncelleme denemeleri JSONL olarak
//! `$XDG_DATA_HOME/updateo/history.jsonl` (varsayılan `~/.local/share/...`)
//! dosyasına eklenir ve `4` sekmesinde en yeni üstte gösterilir.

use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Entry {
    /// Yerel zaman, "YYYY-AA-GG SS:DD"
    pub time: String,
    /// Kaynak etiketi: pacman / AUR / Flatpak
    pub source: String,
    /// "ok" ya da başarısızlık açıklaması
    pub result: String,
    pub pkgs: Vec<String>,
}

/// Yüklenecek en son kayıt sayısı (dosya sonsuza dek büyüyebilir).
const MAX_LOAD: usize = 500;

pub fn history_path() -> PathBuf {
    let base = match std::env::var_os("XDG_DATA_HOME") {
        Some(v) if !v.is_empty() => PathBuf::from(v),
        _ => {
            let home = std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."));
            home.join(".local/share")
        }
    };
    base.join("updateo").join("history.jsonl")
}

/// Uygulama denemesini kaydet (en iyi çaba: loglama güncellemeyi asla
/// düşürmez; yazma hatası sessizce yutulur).
pub fn record(source: &str, result: &str, pkgs: &[String]) {
    let time = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();
    record_to(&history_path(), &time, source, result, pkgs);
}

fn record_to(path: &Path, time: &str, source: &str, result: &str, pkgs: &[String]) {
    let entry = Entry {
        time: time.to_string(),
        source: source.to_string(),
        result: result.to_string(),
        pkgs: pkgs.to_vec(),
    };
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    if let Ok(line) = serde_json::to_string(&entry)
        && let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(path)
    {
        let _ = writeln!(f, "{line}");
    }
}

/// Geçmişi en yeni üstte olacak şekilde yükle.
pub fn load() -> Vec<Entry> {
    load_from(&history_path())
}

fn load_from(path: &Path) -> Vec<Entry> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut v: Vec<Entry> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    if v.len() > MAX_LOAD {
        v.drain(..v.len() - MAX_LOAD);
    }
    v.reverse();
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_newest_first_skips_corrupt() {
        let path = std::env::temp_dir().join(format!(
            "updateo_hist_test_{}_{:?}.jsonl",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_file(&path);
        record_to(&path, "2026-10-26 09:00", "pacman", "ok", &["linux".into()]);
        fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"bu json degil!!!\n")
            .unwrap();
        record_to(
            &path,
            "2026-10-27 10:30",
            "Flatpak",
            "başarısız (exit: 1)",
            &["com.microsoft.Edge".into()],
        );
        let v = load_from(&path);
        assert_eq!(v.len(), 2); // bozuk satır atlanır
        assert_eq!(v[0].time, "2026-10-27 10:30"); // en yeni üstte
        assert_eq!(v[0].source, "Flatpak");
        assert_eq!(v[1].pkgs, ["linux"]);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn missing_file_is_empty() {
        assert!(load_from(Path::new("/nonexistent/updateo/history.jsonl")).is_empty());
    }
}
