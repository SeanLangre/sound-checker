//! Capture from one source, analysis per block, and record / play back.
//!
//! Pulse callbacks run on the GLib main loop, so state is shared with the UI
//! through `Rc<RefCell<..>>` with no locking; the UI only reads the latest
//! snapshot from its frame-synced tick callback.

use std::cell::RefCell;
use std::rc::Rc;

use libpulse_binding as pa;
use libpulse_binding::context::Context;
use libpulse_binding::stream::{FlagSet, PeekResult, State, Stream};

use crate::analysis::Analyzer;
use crate::pulse::{capture_spec, SourceEntry};

pub const RECORD_SECS: u32 = 5;

#[derive(Debug, Clone, PartialEq)]
pub enum CaptureState {
    Connecting,
    Running,
    /// The stream ended (device unplugged, server gone, ...).
    Failed(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum RecordState {
    Idle,
    Recording { frames: usize, target: usize },
    Recorded,
    Playing,
    Error(String),
}

pub struct Shared {
    pub analyzer: Analyzer,
    pub state: CaptureState,
    pub record: RecordState,
    recorded: Vec<f32>,
    rate: u32,
    channels: usize,
}

impl Shared {
    pub fn has_recording(&self) -> bool {
        !self.recorded.is_empty()
    }
}

pub struct Capture {
    pub shared: Rc<RefCell<Shared>>,
    stream: Rc<RefCell<Stream>>,
    playback: RefCell<Option<Rc<RefCell<Stream>>>>,
    ctx: Rc<RefCell<Context>>,
}

impl Capture {
    pub fn start(ctx: &Rc<RefCell<Context>>, source: &SourceEntry) -> Result<Capture, String> {
        let spec = capture_spec(source);
        if !spec.is_valid() {
            return Err("the device reports an unsupported sample format".into());
        }
        let stream = Stream::new(&mut ctx.borrow_mut(), "Sound Checker input", &spec, None)
            .ok_or("could not create a record stream")?;
        let stream = Rc::new(RefCell::new(stream));

        let shared = Rc::new(RefCell::new(Shared {
            analyzer: Analyzer::new(spec.channels as usize, spec.rate),
            state: CaptureState::Connecting,
            record: RecordState::Idle,
            recorded: Vec::new(),
            rate: spec.rate,
            channels: spec.channels as usize,
        }));

        {
            let sh = Rc::clone(&shared);
            let st = Rc::downgrade(&stream);
            stream
                .borrow_mut()
                .set_state_callback(Some(Box::new(move || {
                    let Some(st) = st.upgrade() else { return };
                    let state = st.borrow().get_state();
                    let mut sh = sh.borrow_mut();
                    match state {
                        State::Ready => sh.state = CaptureState::Running,
                        State::Failed => {
                            sh.state = CaptureState::Failed("the capture stream failed".into())
                        }
                        State::Terminated => {
                            sh.state = CaptureState::Failed("the device disappeared".into())
                        }
                        _ => {}
                    }
                })));
        }
        {
            let sh = Rc::clone(&shared);
            let st = Rc::downgrade(&stream);
            stream
                .borrow_mut()
                .set_read_callback(Some(Box::new(move |_len| {
                    let Some(st) = st.upgrade() else { return };
                    drain(&mut st.borrow_mut(), &mut sh.borrow_mut());
                })));
        }

        // ~20 ms fragments keep meters responsive.
        let bytes_per_frame = spec.frame_size() as u32;
        let attr = pa::def::BufferAttr {
            maxlength: u32::MAX,
            tlength: u32::MAX,
            prebuf: u32::MAX,
            minreq: u32::MAX,
            fragsize: (spec.rate / 50).max(1) * bytes_per_frame,
        };
        stream
            .borrow_mut()
            .connect_record(
                Some(&source.name),
                Some(&attr),
                FlagSet::ADJUST_LATENCY | FlagSet::DONT_MOVE,
            )
            .map_err(|e| format!("could not start capture: {e}"))?;

        Ok(Capture {
            shared,
            stream,
            playback: RefCell::new(None),
            ctx: Rc::clone(ctx),
        })
    }

    /// Begin a fixed-length recording from the running stream.
    pub fn start_recording(&self) {
        let mut sh = self.shared.borrow_mut();
        if sh.state != CaptureState::Running {
            return;
        }
        self.stop_playback_inner(&mut sh);
        sh.recorded.clear();
        let target = (sh.rate * RECORD_SECS) as usize;
        sh.record = RecordState::Recording { frames: 0, target };
    }

    /// Play the last recording through the default output.
    pub fn start_playback(&self) {
        let (samples, rate, channels) = {
            let sh = self.shared.borrow();
            if sh.recorded.is_empty() || sh.record == RecordState::Playing {
                return;
            }
            (sh.recorded.clone(), sh.rate, sh.channels)
        };
        let spec = pa::sample::Spec {
            format: pa::sample::Format::F32le,
            rate,
            channels: channels as u8,
        };
        let stream = match Stream::new(&mut self.ctx.borrow_mut(), "Sound Checker playback", &spec, None)
        {
            Some(s) => Rc::new(RefCell::new(s)),
            None => {
                self.shared.borrow_mut().record =
                    RecordState::Error("could not create a playback stream".into());
                return;
            }
        };

        let bytes: Rc<Vec<u8>> = Rc::new(samples.iter().flat_map(|s| s.to_le_bytes()).collect());
        let pos = Rc::new(RefCell::new(0usize));

        {
            let st = Rc::downgrade(&stream);
            let bytes = Rc::clone(&bytes);
            let pos = Rc::clone(&pos);
            stream
                .borrow_mut()
                .set_write_callback(Some(Box::new(move |writable| {
                    let Some(st) = st.upgrade() else { return };
                    let mut p = pos.borrow_mut();
                    let n = writable.min(bytes.len() - *p);
                    if n > 0 {
                        let _ = st
                            .borrow_mut()
                            .write_copy(&bytes[*p..*p + n], 0, pa::stream::SeekMode::Relative);
                        *p += n;
                    }
                    if *p >= bytes.len() {
                        // All data queued; drain lets the tail play out and
                        // then terminates the stream (see state callback).
                        let mut s = st.borrow_mut();
                        s.set_write_callback(None);
                        let _ = s.drain(None);
                    }
                })));
        }
        {
            let sh = Rc::clone(&self.shared);
            let st = Rc::downgrade(&stream);
            stream
                .borrow_mut()
                .set_state_callback(Some(Box::new(move || {
                    let Some(st) = st.upgrade() else { return };
                    let state = st.borrow().get_state();
                    let mut sh = sh.borrow_mut();
                    match state {
                        State::Failed => {
                            sh.record = RecordState::Error("playback failed".into())
                        }
                        State::Terminated if sh.record == RecordState::Playing => {
                            sh.record = RecordState::Recorded
                        }
                        _ => {}
                    }
                })));
        }

        if let Err(e) = stream
            .borrow_mut()
            .connect_playback(None, None, FlagSet::NOFLAGS, None, None)
        {
            self.shared.borrow_mut().record = RecordState::Error(format!("playback failed: {e}"));
            return;
        }
        self.shared.borrow_mut().record = RecordState::Playing;
        *self.playback.borrow_mut() = Some(stream);
    }

    pub fn stop_playback(&self) {
        let mut sh = self.shared.borrow_mut();
        self.stop_playback_inner(&mut sh);
    }

    fn stop_playback_inner(&self, sh: &mut Shared) {
        if let Some(p) = self.playback.borrow_mut().take() {
            let mut p = p.borrow_mut();
            p.set_state_callback(None);
            p.set_write_callback(None);
            let _ = p.disconnect();
        }
        if sh.record == RecordState::Playing {
            sh.record = RecordState::Recorded;
        }
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        {
            let mut sh = self.shared.borrow_mut();
            self.stop_playback_inner(&mut sh);
        }
        let mut s = self.stream.borrow_mut();
        s.set_state_callback(None);
        s.set_read_callback(None);
        if s.get_state().is_good() {
            let _ = s.disconnect();
        }
    }
}

/// Read everything the server has buffered, analyze it and, if a recording is
/// in progress, append it.
fn drain(stream: &mut Stream, sh: &mut Shared) {
    let mut scratch: Vec<f32> = Vec::new();
    loop {
        match stream.peek() {
            Ok(PeekResult::Empty) => break,
            Ok(PeekResult::Hole(_)) => {
                let _ = stream.discard();
            }
            Ok(PeekResult::Data(data)) => {
                scratch.clear();
                scratch.extend(
                    data.chunks_exact(4)
                        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])),
                );
                let _ = stream.discard();
                sh.analyzer.process(&scratch);
                feed_recording(sh, &scratch);
            }
            Err(_) => {
                sh.state = CaptureState::Failed("capture read error".into());
                break;
            }
        }
    }
}

fn feed_recording(sh: &mut Shared, samples: &[f32]) {
    if let RecordState::Recording { frames, target } = sh.record {
        let channels = sh.channels;
        let room = (target - frames) * channels;
        let take = samples.len().min(room);
        sh.recorded.extend_from_slice(&samples[..take]);
        let frames = frames + take / channels;
        sh.record = if frames >= target {
            RecordState::Recorded
        } else {
            RecordState::Recording { frames, target }
        };
    }
}
