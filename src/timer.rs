//! systemd-user zamanlayıcısı: `updateo install-timer` günlük `check --notify`
//! birimi kurar; bildirim `notify-send` ile gider (yoksa günlüğe düşer).
//! Bildirimdeki eylem TUI'yi masaüstü uçbiriminde açar. Saat ve onaysız
//! güncelleme `config.json` içinden okunur.

use crate::config::{self, Config};
use crate::i18n::tr;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

fn unit_dir_from(config_dir: &Path) -> PathBuf {
    config_dir
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("systemd")
        .join("user")
}

fn unit_dir() -> io::Result<PathBuf> {
    Ok(unit_dir_from(&config::dir()?))
}

fn service_text(exe: &str, config: &Config) -> String {
    let action = if config.unattended {
        "update --yes --notify"
    } else {
        "check --notify"
    };
    format!(
        "[Unit]\n\
         Description=updateo package update check\n\
         [Service]\n\
         Type=oneshot\n\
         KillMode=process\n\
         ExecStart=\"{exe}\" {action}\n"
    )
}

fn timer_text(config: &Config) -> String {
    format!(
        "[Unit]\n\
         Description=Daily updateo update check\n\
         [Timer]\n\
         OnCalendar=*-*-* {time}:00\n\
         Persistent=true\n\
         RandomizedDelaySec=900\n\
         [Install]\n\
         WantedBy=timers.target\n",
        time = config.check_time
    )
}

/// systemctl --user çağrısı; hata olursa manuel adım önerilir (sıfır olmayan dönüş).
fn systemctl(args: &[&str]) -> io::Result<bool> {
    match Command::new("systemctl").args(args).status() {
        Ok(st) if st.success() => Ok(true),
        Ok(st) => {
            eprintln!(
                "{}",
                tr(
                    &format!(
                        "systemctl {args:?} failed ({st}); units are written, enable manually"
                    ),
                    &format!(
                        "systemctl {args:?} başarısız ({st}); birimler yazıldı, elle etkinleştirin"
                    ),
                )
            );
            Ok(false)
        }
        Err(e) => {
            eprintln!(
                "{}",
                tr(
                    &format!(
                        "cannot run systemctl: {e}; units are written, run systemctl --user daemon-reload && systemctl --user enable --now updateo.timer"
                    ),
                    &format!(
                        "systemctl çalıştırılamadı: {e}; birimler yazıldı, şu komutu çalıştırın: systemctl --user daemon-reload && systemctl --user enable --now updateo.timer"
                    ),
                )
            );
            Ok(false)
        }
    }
}

pub fn install() -> io::Result<()> {
    let dir = unit_dir()?;
    std::fs::create_dir_all(&dir)?;
    let config = config::load();
    let exe = std::env::current_exe()?
        .canonicalize()
        .unwrap_or_else(|_| std::env::current_exe().unwrap_or_default());
    std::fs::write(
        dir.join("updateo.service"),
        service_text(&exe.display().to_string(), &config),
    )?;
    std::fs::write(dir.join("updateo.timer"), timer_text(&config))?;
    println!(
        "{}",
        tr(
            &format!("units written to {}", dir.display()),
            &format!("birimler yazıldı: {}", dir.display())
        )
    );
    systemctl(&["--user", "daemon-reload"])?;
    let enabled = systemctl(&["--user", "enable", "--now", "updateo.timer"])?;
    if enabled {
        println!(
            "{}",
            tr(
                &format!(
                    "timer installed: daily {} (+up to 15 min jitter), Persistent=true; run 'updateo uninstall-timer' to remove",
                    config.check_time
                ),
                &format!(
                    "zamanlayıcı kuruldu: günlük {} (+15 dk titrek), Persistent=true; kaldırmak için 'updateo uninstall-timer' çalıştırın",
                    config.check_time
                )
            )
        );
    } else {
        println!(
            "{}",
            tr(
                "units written, but the timer is not enabled; enable it manually after fixing systemctl",
                "birimler yazıldı ama zamanlayıcı etkinleştirilmedi; systemctl sorununu giderdikten sonra elle etkinleştirin"
            )
        );
    }
    Ok(())
}

pub fn uninstall() -> io::Result<()> {
    let dir = unit_dir()?;
    let _ = systemctl(&["--user", "disable", "--now", "updateo.timer"]);
    for u in ["updateo.service", "updateo.timer"] {
        let _ = std::fs::remove_file(dir.join(u));
    }
    let _ = systemctl(&["--user", "daemon-reload"]);
    println!(
        "{}",
        tr(
            &format!("timer removed ({})", dir.display()),
            &format!("zamanlayıcı kaldırıldı ({})", dir.display())
        )
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn birim_dizini_xdg_config_altinda() {
        assert_eq!(
            unit_dir_from(Path::new("/home/u/.config/updateo")),
            Path::new("/home/u/.config/systemd/user")
        );
    }

    #[test]
    fn unit_metinleri() {
        let cfg = Config {
            check_time: "07:15".into(),
            unattended: false,
            ..Config::default()
        };
        let svc = service_text("/usr/bin/updateo", &cfg);
        assert!(svc.contains("ExecStart=\"/usr/bin/updateo\" check --notify"));
        assert!(svc.contains("Type=oneshot"));
        // Bildirimden açılan uçbirim, oneshot servis bitince öldürülmemeli.
        assert!(svc.contains("KillMode=process"));
        let unattended = Config {
            unattended: true,
            ..cfg.clone()
        };
        assert!(service_text("/usr/bin/updateo", &unattended).contains("update --yes --notify"));
        let t = timer_text(&cfg);
        assert!(t.contains("OnCalendar=*-*-* 07:15:00") && t.contains("Persistent=true"));
        assert!(t.contains("WantedBy=timers.target"));
    }
}
