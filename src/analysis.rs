//! Per-block audio analysis: meters, waveform decimation and diagnostics.
//!
//! Time is measured in processed frames, not wall-clock, so the analyzer is
//! deterministic and independent of how the server chunks the stream.

use std::collections::VecDeque;

pub const FLOOR_DB: f32 = -96.0;
const WINDOW_SECS: f32 = 0.05;
const PEAK_HOLD_SECS: f32 = 1.5;
const PEAK_FALL_DB_PER_SEC: f32 = 20.0;
const CLIP_LEVEL: f32 = 0.999;
const CLIP_WINDOW_SECS: f32 = 2.0;
const SILENCE_DB: f32 = -70.0;
const SILENCE_AFTER_SECS: f32 = 3.0;
const DEAD_CHANNEL_AFTER_SECS: f32 = 2.0;
const ACTIVE_DB: f32 = -40.0;
const IMBALANCE_DB: f32 = 12.0;
const NOISE_WINDOW_SECS: f32 = 10.0;
pub const WAVEFORM_COLUMNS: usize = 300;
const WAVEFORM_COLUMN_SECS: f32 = 1.0 / 60.0;

pub fn to_db(linear: f32) -> f32 {
    if linear <= 0.0 {
        FLOOR_DB
    } else {
        (20.0 * linear.log10()).max(FLOOR_DB)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Status {
    /// Nothing captured yet.
    Waiting,
    Ok,
    Silence,
    Clipping,
    /// Zero-based index of a channel that carries no signal while others do.
    DeadChannel(usize),
    Imbalanced,
}

#[derive(Debug, Clone, Default)]
pub struct ChannelLevel {
    pub peak_db: f32,
    pub rms_db: f32,
    pub held_peak_db: f32,
}

#[derive(Debug, Clone)]
pub struct Snapshot {
    pub channels: Vec<ChannelLevel>,
    /// Overall RMS across all channels.
    pub rms_db: f32,
    /// Quietest 50 ms window over the last few seconds, if any audio arrived.
    pub noise_floor_db: Option<f32>,
    pub status: Status,
}

pub struct Analyzer {
    channels: usize,
    rate: f32,

    // Current window accumulation.
    win_frames: usize,
    win_len: usize,
    win_peak: Vec<f32>,
    win_sq: Vec<f64>,

    // Last completed window.
    last: Vec<ChannelLevel>,
    last_rms_db: f32,
    have_data: bool,

    // Peak hold, in dB.
    held_db: Vec<f32>,
    held_age: Vec<f32>,

    // Diagnostics, all in seconds of processed audio.
    since_clip: f32,
    silent_for: f32,
    dead_for: Vec<f32>,
    imbalanced_for: f32,
    noise_windows: VecDeque<f32>,

    // Waveform decimation.
    wave: VecDeque<(f32, f32)>,
    col_frames: usize,
    col_len: usize,
    col_min: f32,
    col_max: f32,
}

impl Analyzer {
    pub fn new(channels: usize, rate: u32) -> Self {
        let channels = channels.max(1);
        let rate = rate.max(1) as f32;
        Analyzer {
            channels,
            rate,
            win_frames: 0,
            win_len: ((rate * WINDOW_SECS) as usize).max(1),
            win_peak: vec![0.0; channels],
            win_sq: vec![0.0; channels],
            last: vec![
                ChannelLevel {
                    peak_db: FLOOR_DB,
                    rms_db: FLOOR_DB,
                    held_peak_db: FLOOR_DB
                };
                channels
            ],
            last_rms_db: FLOOR_DB,
            have_data: false,
            held_db: vec![FLOOR_DB; channels],
            held_age: vec![0.0; channels],
            since_clip: f32::INFINITY,
            silent_for: 0.0,
            dead_for: vec![0.0; channels],
            imbalanced_for: 0.0,
            noise_windows: VecDeque::new(),
            wave: VecDeque::with_capacity(WAVEFORM_COLUMNS),
            col_frames: 0,
            col_len: ((rate * WAVEFORM_COLUMN_SECS) as usize).max(1),
            col_min: 0.0,
            col_max: 0.0,
        }
    }

    /// Feed interleaved samples. A trailing partial frame is ignored.
    pub fn process(&mut self, samples: &[f32]) {
        for frame in samples.chunks_exact(self.channels) {
            let mut mix = 0.0;
            for (ch, &s) in frame.iter().enumerate() {
                let a = s.abs();
                if a > self.win_peak[ch] {
                    self.win_peak[ch] = a;
                }
                self.win_sq[ch] += (s as f64) * (s as f64);
                mix += s;
            }
            self.push_wave(mix / self.channels as f32);

            self.win_frames += 1;
            if self.win_frames >= self.win_len {
                self.finish_window();
            }
        }
    }

    fn push_wave(&mut self, s: f32) {
        if self.col_frames == 0 {
            self.col_min = s;
            self.col_max = s;
        } else {
            self.col_min = self.col_min.min(s);
            self.col_max = self.col_max.max(s);
        }
        self.col_frames += 1;
        if self.col_frames >= self.col_len {
            if self.wave.len() == WAVEFORM_COLUMNS {
                self.wave.pop_front();
            }
            self.wave.push_back((self.col_min, self.col_max));
            self.col_frames = 0;
        }
    }

    fn finish_window(&mut self) {
        let dt = self.win_frames as f32 / self.rate;
        let n = self.win_frames as f64;
        let mut total_sq = 0.0;
        let mut any_clip = false;
        let mut max_peak = 0.0f32;

        for ch in 0..self.channels {
            let peak = self.win_peak[ch];
            let rms = (self.win_sq[ch] / n).sqrt() as f32;
            total_sq += self.win_sq[ch];
            any_clip |= peak >= CLIP_LEVEL;
            max_peak = max_peak.max(peak);

            let peak_db = to_db(peak);
            // Peak hold: latch new maxima, hold, then fall.
            self.held_age[ch] += dt;
            if peak_db >= self.held_db[ch] {
                self.held_db[ch] = peak_db;
                self.held_age[ch] = 0.0;
            } else if self.held_age[ch] > PEAK_HOLD_SECS {
                self.held_db[ch] =
                    (self.held_db[ch] - PEAK_FALL_DB_PER_SEC * dt).max(peak_db.max(FLOOR_DB));
            }

            self.last[ch] = ChannelLevel {
                peak_db,
                rms_db: to_db(rms),
                held_peak_db: self.held_db[ch],
            };
            self.win_peak[ch] = 0.0;
            self.win_sq[ch] = 0.0;
        }

        let overall = (total_sq / (n * self.channels as f64)).sqrt() as f32;
        self.last_rms_db = to_db(overall);
        self.have_data = true;
        self.win_frames = 0;

        // Clipping.
        if any_clip {
            self.since_clip = 0.0;
        } else {
            self.since_clip += dt;
        }

        // Silence.
        if to_db(max_peak) < SILENCE_DB {
            self.silent_for += dt;
        } else {
            self.silent_for = 0.0;
        }

        // Dead / imbalanced channels (multi-channel only).
        if self.channels > 1 {
            let loudest = self.last.iter().map(|c| c.rms_db).fold(FLOOR_DB, f32::max);
            for ch in 0..self.channels {
                if loudest > ACTIVE_DB && self.last[ch].peak_db < SILENCE_DB {
                    self.dead_for[ch] += dt;
                } else {
                    self.dead_for[ch] = 0.0;
                }
            }
            let quietest = self.last.iter().map(|c| c.rms_db).fold(0.0, f32::min);
            if loudest > ACTIVE_DB && loudest - quietest > IMBALANCE_DB {
                self.imbalanced_for += dt;
            } else {
                self.imbalanced_for = 0.0;
            }
        }

        // Noise floor: keep the last NOISE_WINDOW_SECS of window RMS values.
        self.noise_windows.push_back(self.last_rms_db);
        let keep = (NOISE_WINDOW_SECS / WINDOW_SECS) as usize;
        while self.noise_windows.len() > keep {
            self.noise_windows.pop_front();
        }
    }

    pub fn snapshot(&self) -> Snapshot {
        let noise_floor_db = self
            .noise_windows
            .iter()
            .copied()
            .reduce(f32::min)
            .filter(|_| self.have_data);
        Snapshot {
            channels: self.last.clone(),
            rms_db: self.last_rms_db,
            noise_floor_db,
            status: self.status(),
        }
    }

    fn status(&self) -> Status {
        if !self.have_data {
            return Status::Waiting;
        }
        if self.silent_for >= SILENCE_AFTER_SECS {
            return Status::Silence;
        }
        if self.since_clip < CLIP_WINDOW_SECS {
            return Status::Clipping;
        }
        if let Some(ch) = self
            .dead_for
            .iter()
            .position(|&t| t >= DEAD_CHANNEL_AFTER_SECS)
        {
            return Status::DeadChannel(ch);
        }
        if self.imbalanced_for >= DEAD_CHANNEL_AFTER_SECS {
            return Status::Imbalanced;
        }
        Status::Ok
    }

    /// Decimated (min, max) columns, oldest first.
    pub fn waveform(&self) -> impl Iterator<Item = (f32, f32)> + '_ {
        self.wave.iter().copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;

    fn tone(frames: usize, channels: usize, amp: &[f32]) -> Vec<f32> {
        let mut v = Vec::with_capacity(frames * channels);
        for i in 0..frames {
            let s = (i as f32 * 0.1).sin();
            for ch in 0..channels {
                v.push(s * amp[ch]);
            }
        }
        v
    }

    fn feed_secs(a: &mut Analyzer, secs: f32, amp: &[f32]) {
        let frames = (RATE as f32 * secs) as usize;
        a.process(&tone(frames, amp.len(), amp));
    }

    #[test]
    fn db_conversion() {
        assert_eq!(to_db(0.0), FLOOR_DB);
        assert!((to_db(1.0)).abs() < 1e-5);
        assert!((to_db(0.5) + 6.02).abs() < 0.01);
    }

    #[test]
    fn waiting_until_first_window() {
        let a = Analyzer::new(2, RATE);
        assert_eq!(a.snapshot().status, Status::Waiting);
        assert!(a.snapshot().noise_floor_db.is_none());
    }

    #[test]
    fn levels_of_half_scale_sine() {
        let mut a = Analyzer::new(1, RATE);
        feed_secs(&mut a, 0.5, &[0.5]);
        let s = a.snapshot();
        assert!((s.channels[0].peak_db + 6.0).abs() < 0.5);
        // RMS of a sine is peak / sqrt(2): about -9 dBFS.
        assert!((s.channels[0].rms_db + 9.0).abs() < 0.5);
        assert_eq!(s.status, Status::Ok);
    }

    #[test]
    fn detects_silence() {
        let mut a = Analyzer::new(2, RATE);
        feed_secs(&mut a, 4.0, &[0.0, 0.0]);
        assert_eq!(a.snapshot().status, Status::Silence);
        feed_secs(&mut a, 0.2, &[0.5, 0.5]);
        assert_eq!(a.snapshot().status, Status::Ok);
    }

    #[test]
    fn detects_clipping_and_recovers() {
        let mut a = Analyzer::new(1, RATE);
        feed_secs(&mut a, 0.2, &[1.5]);
        assert_eq!(a.snapshot().status, Status::Clipping);
        feed_secs(&mut a, 2.5, &[0.3]);
        assert_eq!(a.snapshot().status, Status::Ok);
    }

    #[test]
    fn detects_dead_channel() {
        let mut a = Analyzer::new(2, RATE);
        feed_secs(&mut a, 3.0, &[0.5, 0.0]);
        assert_eq!(a.snapshot().status, Status::DeadChannel(1));
    }

    #[test]
    fn detects_imbalance() {
        let mut a = Analyzer::new(2, RATE);
        feed_secs(&mut a, 3.0, &[0.5, 0.05]);
        assert_eq!(a.snapshot().status, Status::Imbalanced);
    }

    #[test]
    fn mono_never_reports_dead_channel() {
        let mut a = Analyzer::new(1, RATE);
        feed_secs(&mut a, 1.0, &[0.5]);
        assert_eq!(a.snapshot().status, Status::Ok);
    }

    #[test]
    fn noise_floor_is_quietest_window() {
        let mut a = Analyzer::new(1, RATE);
        feed_secs(&mut a, 0.5, &[0.01]);
        feed_secs(&mut a, 0.5, &[0.5]);
        let nf = a.snapshot().noise_floor_db.unwrap();
        assert!(nf < -40.0, "{nf}");
    }

    #[test]
    fn peak_hold_latches_then_falls() {
        let mut a = Analyzer::new(1, RATE);
        feed_secs(&mut a, 0.1, &[0.9]);
        let held = a.snapshot().channels[0].held_peak_db;
        feed_secs(&mut a, 1.0, &[0.01]);
        assert_eq!(a.snapshot().channels[0].held_peak_db, held);
        feed_secs(&mut a, 3.0, &[0.01]);
        assert!(a.snapshot().channels[0].held_peak_db < held - 10.0);
    }

    #[test]
    fn waveform_is_bounded() {
        let mut a = Analyzer::new(1, RATE);
        feed_secs(&mut a, 10.0, &[0.5]);
        assert_eq!(a.waveform().count(), WAVEFORM_COLUMNS);
    }

    #[test]
    fn partial_frames_ignored() {
        let mut a = Analyzer::new(2, RATE);
        a.process(&[0.1, 0.1, 0.1]);
        assert_eq!(a.snapshot().status, Status::Waiting);
    }
}
