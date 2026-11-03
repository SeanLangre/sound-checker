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
        out.push(c);
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
}
