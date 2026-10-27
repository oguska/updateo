//! Kullanıcı yapılandırması: `$XDG_CONFIG_HOME/updateo/config.json`
//! (varsayılan `~/.config/updateo/config.json`). Bootstrap bu dosyayı yazar;
//! TUI, `check`/`update` ve systemd birimleri aynı değerleri okur.

use chrono::NaiveTime;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::PathBuf;

pub const DEFAULT_TIME: &str = "09:30";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(default)]
pub struct Config {
    /// Masaüstü bildirimi gönder.
    pub notify: bool,
    /// Zamanlayıcı güncellemeleri onaysız uygulasın.
    pub unattended: bool,
    /// Günlük kontrol saati, `SS:DD`.
    pub check_time: String,
    /// Oturum açılışında arka plan kontrolü başlat.
    pub autostart: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            notify: true,
            unattended: false,
            check_time: DEFAULT_TIME.into(),
            autostart: false,
        }
    }
}

impl Config {
    /// Geçersiz saat değerini varsayılana çek; elle düzenlenen dosya
    /// yüzünden systemd biriminin bozulmasını engeller.
    pub fn normalize(mut self) -> Self {
        self.check_time = normalize_time(&self.check_time).unwrap_or_else(|| DEFAULT_TIME.into());
        self
    }
}

/// `HH:MM` biçimini doğrula ve iki haneli biçime getir.
pub fn normalize_time(value: &str) -> Option<String> {
    NaiveTime::parse_from_str(value.trim(), "%H:%M")
        .ok()
        .map(|time| time.format("%H:%M").to_string())
}

pub fn dir() -> io::Result<PathBuf> {
    let base = match std::env::var_os("XDG_CONFIG_HOME") {
        Some(v) if !v.is_empty() => PathBuf::from(v),
        _ => match std::env::var_os("HOME") {
            Some(h) if !h.is_empty() => PathBuf::from(h).join(".config"),
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    crate::i18n::tr(
                        "HOME/XDG_CONFIG_HOME is not set",
                        "HOME/XDG_CONFIG_HOME tanımlı değil",
                    ),
                ));
            }
        },
    };
    Ok(base.join("updateo"))
}

pub fn path() -> io::Result<PathBuf> {
    Ok(dir()?.join("config.json"))
}

/// Dosya yoksa/bozuksa varsayılanlarla devam et.
pub fn load() -> Config {
    let Ok(path) = path() else {
        return Config::default();
    };
    let Ok(text) = fs::read_to_string(path) else {
        return Config::default();
    };
    serde_json::from_str::<Config>(&text)
        .unwrap_or_default()
        .normalize()
}

pub fn save(config: &Config) -> io::Result<PathBuf> {
    let dir = dir()?;
    fs::create_dir_all(&dir)?;
    let path = dir.join("config.json");
    let text = serde_json::to_string_pretty(&config.clone().normalize())
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    fs::write(&path, format!("{text}\n"))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saat_dogrulanir_ve_normalize_edilir() {
        assert_eq!(normalize_time("9:05").as_deref(), Some("09:05"));
        assert_eq!(normalize_time("23:59").as_deref(), Some("23:59"));
        assert_eq!(normalize_time("24:00"), None);
        assert_eq!(normalize_time("09:5").as_deref(), Some("09:05"));
    }

    #[test]
    fn gecersiz_saat_varsayilana_doner() {
        let cfg = Config {
            check_time: "99:99".into(),
            ..Config::default()
        }
        .normalize();
        assert_eq!(cfg.check_time, DEFAULT_TIME);
    }

    #[test]
    fn json_gidis_donusu() {
        let cfg = Config {
            notify: false,
            unattended: true,
            check_time: "07:15".into(),
            autostart: true,
        };
        let text = serde_json::to_string(&cfg).unwrap();
        assert_eq!(serde_json::from_str::<Config>(&text).unwrap(), cfg);
        // eksik alanlar varsayılanla tamamlanır
        let partial: Config = serde_json::from_str(r#"{"notify":false}"#).unwrap();
        assert!(!partial.notify);
        assert_eq!(partial.check_time, DEFAULT_TIME);
    }
}
