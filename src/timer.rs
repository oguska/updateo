//! systemd-user zamanlayıcısı: `updateo install-timer` günlük `check --notify`
//! birimi kurar; bildirim `notify-send` ile gider (yoksa günlüğe düşer).
//! Bildirimdeki eylem TUI'yi masaüstü uçbiriminde açar.

use crate::i18n::tr;
use std::io;
use std::path::PathBuf;
use std::process::Command;

fn unit_dir() -> io::Result<PathBuf> {
    let base = match std::env::var_os("XDG_CONFIG_HOME") {
        Some(v) if !v.is_empty() => PathBuf::from(v),
        _ => match std::env::var_os("HOME") {
            Some(h) if !h.is_empty() => PathBuf::from(h).join(".config"),
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    tr(
                        "HOME/XDG_CONFIG_HOME is not set",
                        "HOME/XDG_CONFIG_HOME tanımlı değil",
                    ),
                ));
            }
        },
    };
    Ok(base.join("systemd").join("user"))
}

fn service_text(exe: &str) -> String {
    format!(
        "[Unit]\n\
         Description=updateo package update check\n\
         [Service]\n\
         Type=oneshot\n\
         KillMode=process\n\
         ExecStart=\"{exe}\" check --notify\n"
    )
}

fn timer_text() -> String {
    "[Unit]\n\
     Description=Daily updateo update check\n\
     [Timer]\n\
     OnCalendar=*-*-* 09:30:00\n\
     Persistent=true\n\
     RandomizedDelaySec=900\n\
     [Install]\n\
     WantedBy=timers.target\n"
        .to_string()
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
    let exe = std::env::current_exe()?
        .canonicalize()
        .unwrap_or_else(|_| std::env::current_exe().unwrap_or_default());
    std::fs::write(
        dir.join("updateo.service"),
        service_text(&exe.display().to_string()),
    )?;
    std::fs::write(dir.join("updateo.timer"), timer_text())?;
    println!(
        "{}",
        tr(
            &format!("units written to {}", dir.display()),
            &format!("birimler yazıldı: {}", dir.display())
        )
    );
    systemctl(&["--user", "daemon-reload"])?;
    systemctl(&["--user", "enable", "--now", "updateo.timer"])?;
    println!(
        "{}",
        tr(
            "timer installed: daily 09:30 (+up to 15 min jitter), Persistent=true; run 'updateo uninstall-timer' to remove",
            "zamanlayıcı kuruldu: günlük 09:30 (+15 dk titrek), Persistent=true; kaldırmak için 'updateo uninstall-timer' çalıştırın"
        )
    );
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
    fn unit_metinleri() {
        let svc = service_text("/usr/bin/updateo");
        assert!(svc.contains("ExecStart=\"/usr/bin/updateo\" check --notify"));
        assert!(svc.contains("Type=oneshot"));
        // Bildirimden açılan uçbirim, oneshot servis bitince öldürülmemeli.
        assert!(svc.contains("KillMode=process"));
        let t = timer_text();
        assert!(t.contains("OnCalendar=*-*-* 09:30:00") && t.contains("Persistent=true"));
        assert!(t.contains("WantedBy=timers.target"));
    }
}
