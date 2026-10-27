//! Güncelleme geçmişi: uygulanan güncelleme denemeleri SQLite veritabanında
//! `$XDG_DATA_HOME/updateo/history.db` (varsayılan `~/.local/share/...`)
//! tutulur ve `4` sekmesinde en yeni üstte gösterilir. Eski sürümlerin
//! `history.jsonl` dosyası ilk açılışta bir kez içe aktarılır.

use rusqlite::Connection;
use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Deserialize, Clone, Debug, PartialEq)]
pub struct Entry {
    /// Yerel zaman, "YYYY-AA-GG SS:DD"
    pub time: String,
    /// Kaynak etiketi: pacman / AUR / Flatpak
    pub source: String,
    /// "ok" ya da başarısızlık açıklaması
    pub result: String,
    pub pkgs: Vec<String>,
}

/// Yüklenecek en son kayıt sayısı (veritabanı sonsuza dek büyüyebilir).
const MAX_LOAD: i64 = 500;

fn data_dir(file: &str) -> PathBuf {
    let base = match std::env::var_os("XDG_DATA_HOME") {
        Some(v) if !v.is_empty() => PathBuf::from(v),
        _ => {
            let home = std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."));
            home.join(".local/share")
        }
    };
    base.join("updateo").join(file)
}

pub fn db_path() -> PathBuf {
    data_dir("history.db")
}

fn jsonl_path() -> PathBuf {
    data_dir("history.jsonl")
}

fn open(db: &Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open(db)?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS history(
             id     INTEGER PRIMARY KEY,
             time   TEXT NOT NULL,
             source TEXT NOT NULL,
             result TEXT NOT NULL,
             pkgs   TEXT NOT NULL DEFAULT ''
         );",
    )?;
    Ok(conn)
}

/// Eski JSONL günlüğünü bir kez içe aktar; dosya `history.jsonl.bak` olur.
fn migrate_jsonl(db: &Path, jsonl: &Path) {
    if db.exists() || !jsonl.exists() {
        return;
    }
    if let Some(dir) = db.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let Ok(text) = fs::read_to_string(jsonl) else {
        return;
    };
    let Ok(conn) = open(db) else { return };
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        if let Ok(e) = serde_json::from_str::<Entry>(line) {
            let _ = conn.execute(
                "INSERT INTO history(time, source, result, pkgs) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![e.time, e.source, e.result, e.pkgs.join(" ")],
            );
        }
    }
    let _ = fs::rename(jsonl, jsonl.with_extension("jsonl.bak"));
}

/// Uygulama denemesini kaydet (en iyi çaba: loglama güncellemeyi asla
/// düşürmez; yazma hatası sessizce yutulur).
pub fn record(source: &str, result: &str, pkgs: &[String]) {
    let time = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();
    let db = db_path();
    migrate_jsonl(&db, &jsonl_path());
    let _ = record_to(&db, &time, source, result, pkgs);
}

fn record_to(
    db: &Path,
    time: &str,
    source: &str,
    result: &str,
    pkgs: &[String],
) -> rusqlite::Result<()> {
    if let Some(dir) = db.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let conn = open(db)?;
    conn.execute(
        "INSERT INTO history(time, source, result, pkgs) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![time, source, result, pkgs.join(" ")],
    )?;
    Ok(())
}

/// Kayıt başarıyla mı sonuçlandı? Başarı etiketleri sabit: `ok`
/// (güncelleme/kurma) ve `removed` (kaldırma); gerisi başarısızlık sayılır.
pub fn is_success(result: &str) -> bool {
    matches!(result, "ok" | "removed")
}

/// Geçmişi en yeni üstte olacak şekilde yükle.
pub fn load() -> Vec<Entry> {
    let db = db_path();
    migrate_jsonl(&db, &jsonl_path());
    load_from(&db)
}

fn load_from(db: &Path) -> Vec<Entry> {
    let Ok(conn) = open(db) else {
        return Vec::new();
    };
    let Ok(mut stmt) =
        conn.prepare("SELECT time, source, result, pkgs FROM history ORDER BY id DESC LIMIT ?1")
    else {
        return Vec::new();
    };
    let rows = stmt.query_map([MAX_LOAD], |r| {
        let pkgs: String = r.get(3)?;
        Ok(Entry {
            time: r.get(0)?,
            source: r.get(1)?,
            result: r.get(2)?,
            pkgs: pkgs
                .split(' ')
                .filter(|p| !p.is_empty())
                .map(String::from)
                .collect(),
        })
    });
    rows.map(|it| it.filter_map(|e| e.ok()).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_db(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "updateo_hist_{}_{:?}_{tag}.db",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_file(&p);
        p
    }

    #[test]
    fn roundtrip_newest_first() {
        let db = tmp_db("rt");
        record_to(&db, "2026-10-26 09:00", "pacman", "ok", &["linux".into()]).unwrap();
        record_to(
            &db,
            "2026-10-27 10:30",
            "Flatpak",
            "failed (exit: 1)",
            &["com.microsoft.Edge".into()],
        )
        .unwrap();
        let v = load_from(&db);
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].time, "2026-10-27 10:30"); // en yeni üstte
        assert_eq!(v[0].source, "Flatpak");
        assert_eq!(v[1].pkgs, ["linux"]);
        let _ = fs::remove_file(&db);
    }

    #[test]
    fn jsonl_migration_once() {
        let dir = std::env::temp_dir().join(format!("updateo_mig_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let db = dir.join("history.db");
        let jsonl = dir.join("history.jsonl");
        fs::write(
            &jsonl,
            "{\"time\":\"2026-10-01 10:00\",\"source\":\"pacman\",\"result\":\"ok\",\"pkgs\":[\"glibc\"]}\nbu json degil!!!\n",
        )
        .unwrap();
        migrate_jsonl(&db, &jsonl);
        migrate_jsonl(&db, &jsonl); // ikinci kez çalışmaz
        let v = load_from(&db);
        assert_eq!(v.len(), 1); // bozuk satır atlandı
        assert_eq!(v[0].pkgs, ["glibc"]);
        assert!(!jsonl.exists() && jsonl.with_extension("jsonl.bak").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_db_is_empty() {
        assert!(load_from(Path::new("/nonexistent/updateo/history.db")).is_empty());
    }
}
