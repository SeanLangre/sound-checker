//! Start-at-login toggle: an XDG autostart entry that runs this binary (the
//! AppImage when launched from one) with `--background`.

use std::fs;
use std::path::PathBuf;

fn entry_path() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from(".config"))
        .join("autostart")
        .join("dev.soundchecker.SoundChecker.desktop")
}

/// Quote a path for a desktop-entry `Exec` value.
fn quote(path: &str) -> String {
    let mut out = String::from("\"");
    for c in path.chars() {
        if matches!(c, '"' | '`' | '$' | '\\') {
            out.push('\\');
        }
        // `%` starts a field code (%f, %u, ...) unless doubled.
        if c == '%' {
            out.push('%');
        }
        // A raw line break would end the Exec line and start new keys.
        match c {
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn contents(exe: &str) -> String {
    format!(
        "[Desktop Entry]\nType=Application\nName=Linux Sound Checker\n\
         Comment=Reconnect Bluetooth audio devices and sit in the tray\n\
         Exec={} --background\nIcon=audio-input-microphone\nTerminal=false\n\
         X-GNOME-Autostart-enabled=true\n",
        quote(exe)
    )
}

pub fn is_enabled() -> bool {
    fs::read_to_string(entry_path())
        .is_ok_and(|t| !t.lines().any(|l| l == "Hidden=true" || l == "X-GNOME-Autostart-enabled=false"))
}

pub fn set_enabled(on: bool) -> std::io::Result<()> {
    let path = entry_path();
    if !on {
        return match fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        };
    }
    // $APPIMAGE is the real file; current_exe would be a temporary mount.
    let exe = std::env::var("APPIMAGE")
        .ok()
        .or_else(|| std::env::current_exe().ok().map(|p| p.to_string_lossy().into_owned()))
        .ok_or_else(|| std::io::Error::other("cannot find own path"))?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(path, contents(&exe))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exec_line_is_quoted() {
        let c = contents("/home/a b/App\"x.AppImage");
        assert!(c.contains("Exec=\"/home/a b/App\\\"x.AppImage\" --background\n"));
    }

    #[test]
    fn fuzz_exec_quoting_is_safe_for_any_path() {
        use crate::fuzz::{iters, Rng};
        let mut rng = Rng::new(10);
        for case in 0..iters(5000) {
            let path = rng.string(20);
            let entry = contents(&path);
            // The Exec line must stay a single line of the file.
            let exec: Vec<&str> = entry.lines().filter(|l| l.starts_with("Exec=")).collect();
            assert_eq!(exec.len(), 1, "case {case}: path {path:?} injected/broke lines");
            // No stray keys injected via the path.
            assert_eq!(entry.lines().count(), 8, "case {case}: path {path:?}");
            // Desktop-entry field codes (%f, %u, ...) must be neutralised.
            let line = exec[0];
            let bytes = line.as_bytes();
            let mut i = 0;
            while i < bytes.len() {
                if bytes[i] == b'%' {
                    assert_eq!(bytes.get(i + 1), Some(&b'%'), "case {case}: unescaped % in {line:?}");
                    i += 1;
                }
                i += 1;
            }
        }
    }
}
