//! Reconnect previously used Bluetooth audio devices at login.
//!
//! The app remembers Bluetooth audio devices it has seen connected
//! (`~/.config/sound-checker/bluetooth.json`). `sound-checker --reconnect`
//! (run from the autostart entry) asks BlueZ to connect the ones that are not
//! connected yet, retrying for a while because the adapter and the headphones
//! may not be ready right after login, then exits.

use std::fs;
use std::io;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Known {
    pub mac: String,
    pub name: String,
}

fn state_path() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from(".config"))
        .join("sound-checker")
        .join("bluetooth.json")
}

fn load() -> Option<Vec<Known>> {
    serde_json::from_slice(&fs::read(state_path()).ok()?).ok()
}

fn save(list: &[Known]) -> io::Result<()> {
    let path = state_path();
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(path, serde_json::to_vec_pretty(list).map_err(io::Error::other)?)
}

/// `bluez_card.74_B7_E6_D6_60_0D` -> `74:B7:E6:D6:60:0D`.
pub fn mac_from_card_name(name: &str) -> Option<String> {
    let raw = name.strip_prefix("bluez_card.")?;
    let parts: Vec<&str> = raw.split('_').collect();
    let ok = parts.len() == 6 && parts.iter().all(|p| p.len() == 2 && p.chars().all(|c| c.is_ascii_hexdigit()));
    ok.then(|| parts.join(":").to_uppercase())
}

/// Add devices to the remembered list; returns whether anything changed.
fn merge(list: &mut Vec<Known>, seen: &[Known]) -> bool {
    let mut changed = false;
    for s in seen {
        match list.iter_mut().find(|k| k.mac == s.mac) {
            Some(k) if k.name != s.name => {
                k.name = s.name.clone();
                changed = true;
            }
            Some(_) => {}
            None => {
                list.push(s.clone());
                changed = true;
            }
        }
    }
    changed
}

/// Remember Bluetooth audio devices the audio server currently shows.
pub fn remember(seen: &[Known]) {
    if seen.is_empty() {
        return;
    }
    let mut list = load().unwrap_or_default();
    if merge(&mut list, seen) {
        let _ = save(&list);
    }
}

fn bluetoothctl(args: &[&str]) -> String {
    Command::new("bluetoothctl")
        .args(args)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

fn parse_devices(text: &str) -> Vec<Known> {
    text.lines()
        .filter_map(|l| l.strip_prefix("Device "))
        .filter_map(|l| {
            let (mac, name) = l.split_once(' ')?;
            Some(Known { mac: mac.to_string(), name: name.trim().to_string() })
        })
        .collect()
}

fn is_audio(info: &str) -> bool {
    info.lines().any(|l| l.trim().starts_with("Icon: audio-"))
}

fn is_connected(mac: &str) -> bool {
    bluetoothctl(&["info", mac]).contains("Connected: yes")
}

/// First run: nothing remembered yet, so start from the paired headsets and
/// headphones (not phones, mice or controllers).
fn seed() -> Vec<Known> {
    parse_devices(&bluetoothctl(&["devices", "Paired"]))
        .into_iter()
        .filter(|d| is_audio(&bluetoothctl(&["info", &d.mac])))
        .collect()
}

static SCANNING: AtomicBool = AtomicBool::new(false);

/// Whether a scan is in progress.
pub fn scanning() -> bool {
    SCANNING.load(Ordering::SeqCst)
}

/// Run a scan on a background thread unless one is already running.
pub fn start_scan() {
    if !SCANNING.swap(true, Ordering::SeqCst) {
        std::thread::spawn(|| {
            run();
            SCANNING.store(false, Ordering::SeqCst);
        });
    }
}

/// How long a scan keeps trying, checking once per second.
const SCAN_SECS: u64 = 30;

/// Connect remembered devices that are not connected. Blocking.
/// Checks every second for 30 seconds: the adapter may still be powering up,
/// and headphones that are off simply refuse until they are switched on.
pub fn run() {
    let mut pending = load().unwrap_or_else(|| {
        let s = seed();
        let _ = save(&s);
        s
    });
    let deadline = Instant::now() + Duration::from_secs(SCAN_SECS);
    loop {
        let started = Instant::now();
        pending.retain(|d| !is_connected(&d.mac));
        if pending.is_empty() {
            return;
        }
        for d in &pending {
            let _ = Command::new("bluetoothctl").args(["--timeout", "3", "connect", &d.mac]).output();
        }
        let next = started + Duration::from_secs(1);
        if next >= deadline {
            return;
        }
        std::thread::sleep(next.saturating_duration_since(Instant::now()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(mac: &str, name: &str) -> Known {
        Known { mac: mac.into(), name: name.into() }
    }

    #[test]
    fn card_name_to_mac() {
        assert_eq!(mac_from_card_name("bluez_card.74_B7_E6_D6_60_0D").as_deref(), Some("74:B7:E6:D6:60:0D"));
        assert_eq!(mac_from_card_name("alsa_card.pci-0000_00_1f.3"), None);
        assert_eq!(mac_from_card_name("bluez_card.nope"), None);
    }

    #[test]
    fn parses_devices_with_spaces_in_names() {
        let d = parse_devices("Device 30:4C:3C:32:4C:13 H&S V\nDevice 74:B7:E6:D6:60:0D WI-C100\n");
        assert_eq!(d, [k("30:4C:3C:32:4C:13", "H&S V"), k("74:B7:E6:D6:60:0D", "WI-C100")]);
    }

    #[test]
    fn merge_adds_and_renames_once() {
        let mut list = vec![k("A", "old")];
        assert!(merge(&mut list, &[k("A", "new"), k("B", "b")]));
        assert_eq!(list, [k("A", "new"), k("B", "b")]);
        assert!(!merge(&mut list, &[k("A", "new")]));
    }

    #[test]
    fn audio_icon_detection() {
        assert!(is_audio("\tIcon: audio-headset\n"));
        assert!(!is_audio("\tIcon: phone\n"));
    }

    #[test]
    fn fuzz_parsers_never_panic_and_stay_well_formed() {
        use crate::fuzz::{iters, Rng};
        let mut rng = Rng::new(5);
        for case in 0..iters(5000) {
            let text = rng.string(40);
            for d in parse_devices(&text) {
                assert!(!d.mac.contains(' '), "case {case}");
                assert_eq!(d.name, d.name.trim(), "case {case}");
            }
            let _ = is_audio(&text);
            if let Some(mac) = mac_from_card_name(&text) {
                let parts: Vec<&str> = mac.split(':').collect();
                assert_eq!(parts.len(), 6);
                assert!(parts.iter().all(|p| p.len() == 2 && p.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_lowercase())));
            }
        }
        // Valid names must always convert.
        for _ in 0..iters(500) {
            let hex: Vec<String> = (0..6).map(|_| format!("{:02x}", rng.below(256))).collect();
            let mac = mac_from_card_name(&format!("bluez_card.{}", hex.join("_"))).expect("valid card name");
            assert_eq!(mac, hex.join(":").to_uppercase());
        }
    }

    #[test]
    fn fuzz_merge_is_idempotent_and_keeps_macs_unique() {
        use crate::fuzz::{iters, Rng};
        let mut rng = Rng::new(6);
        let mk = |rng: &mut Rng| -> Vec<Known> {
            let mut v: Vec<Known> = (0..rng.below(8)).map(|_| k(&format!("M{}", rng.below(5)), &format!("n{}", rng.below(3)))).collect();
            // A device list from the server has one entry per MAC.
            v.sort_by(|a, b| a.mac.cmp(&b.mac));
            v.dedup_by(|a, b| a.mac == b.mac);
            v
        };
        for case in 0..iters(2000) {
            let mut list: Vec<Known> = Vec::new();
            merge(&mut list, &mk(&mut rng));
            let seen = mk(&mut rng);
            merge(&mut list, &seen);
            let snapshot = list.clone();
            assert!(!merge(&mut list, &seen), "second merge must be a no-op, case {case}");
            assert_eq!(list, snapshot);
            let mut macs: Vec<_> = list.iter().map(|k| &k.mac).collect();
            macs.sort();
            let n = macs.len();
            macs.dedup();
            assert_eq!(n, macs.len(), "duplicate MACs, case {case}");
        }
    }
}
