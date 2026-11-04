//! A short Mozart phrase looped on the default output, so the user can hear which
//! device audio is going to after a restart or rescan.

use std::f32::consts::TAU;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

const RATE: u32 = 44_100;
const D4: f32 = 293.66;
const G4: f32 = 392.00;
const A4: f32 = 440.00;
const B4: f32 = 493.88;
const D5: f32 = 587.33;
const FS5: f32 = 739.99;
const A5: f32 = 880.00;
/// One eighth note; the four phrases below last about 10 seconds.
const EIGHTH_SECS: f32 = 0.16;

/// One two-bar phrase of the opening of Mozart's "Eine kleine Nachtmusik"
/// (16 eighths), as (pitch, start, length) in eighths. `main` is the
/// repeated note, `low` the one it alternates with, `third` the step up
/// before the long `top` note.
fn phrase(base: u32, main: f32, low: f32, third: f32, top: f32) -> [(f32, u32, u32); 9] {
    [
        (main, base, 2),
        (low, base + 3, 1),
        (main, base + 4, 2),
        (low, base + 7, 1),
        (main, base + 8, 1),
        (low, base + 9, 1),
        (main, base + 10, 1),
        (third, base + 11, 1),
        (top, base + 12, 4),
    ]
}

/// The whole melody: phrase in G, answer in D, then both again.
fn melody() -> Vec<(f32, u32, u32)> {
    let mut v = Vec::new();
    for round in 0..2 {
        v.extend(phrase(round * 32, G4, D4, B4, D5));
        v.extend(phrase(round * 32 + 16, D5, A4, FS5, A5));
    }
    v
}
/// Extra ring after a note's written length.
const TAIL_SECS: f32 = 0.5;

/// 16-bit mono PCM WAV of the melody.
fn wav() -> Vec<u8> {
    let eighth = (RATE as f32 * EIGHTH_SECS) as usize;
    let tail = (RATE as f32 * TAIL_SECS) as usize;
    let notes = melody();
    let total = notes.iter().map(|&(_, st, len)| (st + len) as usize * eighth + tail).max().unwrap_or(0);
    let mut mix = vec![0.0f32; total];
    for &(f, start, len) in &notes {
        let n = len as usize * eighth + tail;
        for i in 0..n {
            let t = i as f32 / RATE as f32;
            // Plucked, music-box-like: fast attack, smooth decay.
            let env = (i as f32 / (RATE as f32 * 0.01)).min(1.0) * (-t * 4.0).exp();
            let tone = (t * f * TAU).sin() + 0.3 * (t * f * 2.0 * TAU).sin() + 0.1 * (t * f * 3.0 * TAU).sin();
            mix[start as usize * eighth + i] += tone * env * 0.3;
        }
    }
    let pcm: Vec<u8> = mix
        .iter()
        .flat_map(|v| ((v.clamp(-1.0, 1.0) * i16::MAX as f32) as i16).to_le_bytes())
        .collect();
    let mut out = Vec::with_capacity(44 + pcm.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + pcm.len() as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&RATE.to_le_bytes());
    out.extend_from_slice(&(RATE * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
    out.extend_from_slice(&pcm);
    out
}

/// Bumped to start a new loop or stop the current one.
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// Generation of the loop that is playing now; 0 when none is.
static PLAYING: AtomicU64 = AtomicU64::new(0);

/// Whether the jingle loop is currently running.
pub fn playing() -> bool {
    PLAYING.load(Ordering::SeqCst) != 0
}

/// How long the loop keeps playing unless stopped.
const LOOP_SECS: u64 = 30;
/// Pause between repeats; also the retry delay while the output is changing.
const GAP: Duration = Duration::from_millis(150);

/// Stop the jingle, if one is playing.
pub fn stop() {
    GENERATION.fetch_add(1, Ordering::SeqCst);
    PLAYING.store(0, Ordering::SeqCst);
}

/// Loop the jingle on the default output for 30 seconds, replacing any loop
/// already running. Does not block. If playback fails (the output is being
/// restarted) it keeps retrying, so it picks up on whichever device comes back.
pub fn play_loop() {
    let gen = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    PLAYING.store(gen, Ordering::SeqCst);
    std::thread::spawn(move || {
        let path = std::env::temp_dir().join(format!("sound-checker-chime-{}-{gen}.wav", std::process::id()));
        if std::fs::write(&path, wav()).is_err() {
            let _ = PLAYING.compare_exchange(gen, 0, Ordering::SeqCst, Ordering::SeqCst);
            return;
        }
        let current = || GENERATION.load(Ordering::SeqCst) == gen;
        let deadline = Instant::now() + Duration::from_secs(LOOP_SECS);
        let mut player = 0;
        while current() && Instant::now() < deadline {
            let child = Command::new(["paplay", "pw-play"][player])
                .arg(&path)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn();
            match child {
                Ok(mut c) => loop {
                    match c.try_wait() {
                        Ok(None) if current() => std::thread::sleep(Duration::from_millis(50)),
                        Ok(None) => {
                            let _ = c.kill();
                            let _ = c.wait();
                            break;
                        }
                        _ => break,
                    }
                },
                // Player not installed: try the next one, give up after both.
                Err(_) if player == 0 => {
                    player = 1;
                    continue;
                }
                Err(_) => break,
            }
            let until = Instant::now() + GAP;
            while current() && Instant::now() < until {
                std::thread::sleep(Duration::from_millis(50));
            }
        }
        let _ = std::fs::remove_file(&path);
        let _ = PLAYING.compare_exchange(gen, 0, Ordering::SeqCst, Ordering::SeqCst);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_header_matches_data() {
        let w = wav();
        let data_len = u32::from_le_bytes(w[40..44].try_into().unwrap()) as usize;
        assert_eq!(w.len(), 44 + data_len);
        assert_eq!(&w[0..4], b"RIFF");
    }
}
