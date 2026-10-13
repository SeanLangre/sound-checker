//! User-chosen display names for devices, kept only by this app.
//!
//! Stored in `~/.config/sound-checker/names.json`, keyed by the stable
//! PipeWire node name, so a name survives unplugging and replugging.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::PathBuf;

pub type Names = HashMap<String, String>;

fn path() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from(".config"))
        .join("sound-checker")
        .join("names.json")
}

pub fn load() -> Names {
    fs::read(path())
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn save(names: &Names) -> io::Result<()> {
    let path = path();
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(path, serde_json::to_vec_pretty(names).map_err(io::Error::other)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip_through_json() {
        let mut n = Names::new();
        n.insert("alsa_input.x".into(), "Podcast mic".into());
        let json = serde_json::to_vec(&n).unwrap();
        assert_eq!(serde_json::from_slice::<Names>(&json).unwrap(), n);
    }
}
