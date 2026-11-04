//! Launch host tools (`bluetoothctl`, `pactl`, ...) without the AppImage's
//! bundled-library environment. `AppRun` points `LD_LIBRARY_PATH` at libraries
//! bundled from the build distro; a host binary that inherits that can load
//! mismatched libs and fail to start (seen on Fedora), silently.

use std::process::Command;

/// `LD_LIBRARY_PATH` without the entries inside `appdir`; `None` if none remain.
fn without_appdir(value: &str, appdir: &str) -> Option<String> {
    let appdir = appdir.trim_end_matches('/');
    let kept: Vec<&str> = value
        .split(':')
        .filter(|p| !p.is_empty() && *p != appdir && !p.starts_with(&format!("{appdir}/")))
        .collect();
    (!kept.is_empty()).then(|| kept.join(":"))
}

pub fn command(program: &str) -> Command {
    let mut cmd = Command::new(program);
    if let Some(appdir) = std::env::var("APPDIR").ok().filter(|a| !a.is_empty()) {
        match std::env::var("LD_LIBRARY_PATH").ok().and_then(|v| without_appdir(&v, &appdir)) {
            Some(v) => cmd.env("LD_LIBRARY_PATH", v),
            None => cmd.env_remove("LD_LIBRARY_PATH"),
        };
        for var in ["GIO_MODULE_DIR", "GSETTINGS_SCHEMA_DIR", "GDK_PIXBUF_MODULE_FILE"] {
            cmd.env_remove(var);
        }
    }
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_only_appdir_entries() {
        let app = "/tmp/.mount_abc";
        assert_eq!(without_appdir("/tmp/.mount_abc/usr/lib", app), None);
        assert_eq!(without_appdir("/tmp/.mount_abc/usr/lib:/opt/x", app).as_deref(), Some("/opt/x"));
        assert_eq!(without_appdir("/opt/x:/tmp/.mount_abc/usr/lib:/opt/y", app).as_deref(), Some("/opt/x:/opt/y"));
        // A sibling directory sharing the prefix is not inside the AppDir.
        assert_eq!(without_appdir("/tmp/.mount_abcd/lib", app).as_deref(), Some("/tmp/.mount_abcd/lib"));
        assert_eq!(without_appdir("", app), None);
    }

    #[test]
    fn fuzz_never_panics_and_never_keeps_appdir_entries() {
        use crate::fuzz::{iters, Rng};
        let mut rng = Rng::new(11);
        for case in 0..iters(5000) {
            let app = format!("/{}", rng.string(6));
            let value = (0..rng.below(5)).map(|_| if rng.chance(50) { format!("{app}/{}", rng.string(4)) } else { rng.string(8) }).collect::<Vec<_>>().join(":");
            if let Some(out) = without_appdir(&value, &app) {
                assert!(out.split(':').all(|p| !p.starts_with(&format!("{app}/"))), "case {case}");
            }
        }
    }
}
