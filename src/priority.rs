//! Device priority: when a device appears, the highest-ranked one present
//! becomes the default.
//!
//! Stored in `~/.config/sound-checker/priorities.json` as ordered lists of
//! device names (first = highest), so the order survives unplugging.

use std::fs;
use std::io;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Priorities {
    #[serde(default)]
    pub inputs: Vec<String>,
    #[serde(default)]
    pub outputs: Vec<String>,
}

impl Priorities {
    pub fn list_mut(&mut self, output: bool) -> &mut Vec<String> {
        if output { &mut self.outputs } else { &mut self.inputs }
    }
}

fn path() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from(".config"))
        .join("sound-checker")
        .join("priorities.json")
}

pub fn load() -> Priorities {
    fs::read(path())
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn save(p: &Priorities) -> io::Result<()> {
    let path = path();
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(path, serde_json::to_vec_pretty(p).map_err(io::Error::other)?)
}

/// Position in `list`; unranked devices sort after every ranked one.
pub fn rank(list: &[String], name: &str) -> usize {
    list.iter().position(|n| n == name).unwrap_or(usize::MAX)
}

/// The ranked candidate with the highest priority, if any is ranked.
pub fn best<'a>(list: &[String], candidates: impl Iterator<Item = &'a str>) -> Option<&'a str> {
    candidates
        .filter(|c| rank(list, c) != usize::MAX)
        .min_by_key(|c| rank(list, c))
}

/// Move `name` one step up or down among the devices in `visible` (given in
/// their current display order). Unranked visible devices are first appended
/// to `list` in that order, so the displayed order is what gets edited.
pub fn shift(list: &mut Vec<String>, visible: &[String], name: &str, up: bool) {
    let missing: Vec<String> = visible.iter().filter(|v| !list.contains(v)).cloned().collect();
    list.extend(missing);
    let Some(pos) = list.iter().position(|n| n == name) else { return };
    let is_visible = |n: &String| visible.contains(n);
    let other = if up {
        (0..pos).rev().find(|&i| is_visible(&list[i]))
    } else {
        (pos + 1..list.len()).find(|&i| is_visible(&list[i]))
    };
    if let Some(o) = other {
        list.swap(pos, o);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &[&str]) -> Vec<String> {
        s.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn best_picks_highest_ranked_present() {
        let list = v(&["a", "b", "c"]);
        assert_eq!(best(&list, ["c", "b", "x"].into_iter()), Some("b"));
        assert_eq!(best(&list, ["x"].into_iter()), None);
        assert_eq!(best(&[], ["a"].into_iter()), None);
    }

    #[test]
    fn shift_materialises_visible_order() {
        let mut list = Vec::new();
        shift(&mut list, &v(&["a", "b", "c"]), "c", true);
        assert_eq!(list, v(&["a", "c", "b"]));
    }

    #[test]
    fn shift_skips_absent_devices() {
        let mut list = v(&["a", "gone", "b"]);
        shift(&mut list, &v(&["a", "b"]), "b", true);
        assert_eq!(list, v(&["b", "gone", "a"]));
    }

    #[test]
    fn shift_at_edge_is_noop() {
        let mut list = v(&["a", "b"]);
        shift(&mut list, &v(&["a", "b"]), "a", true);
        assert_eq!(list, v(&["a", "b"]));
        shift(&mut list, &v(&["a", "b"]), "b", false);
        assert_eq!(list, v(&["a", "b"]));
    }
}
