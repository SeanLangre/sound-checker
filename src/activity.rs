//! "Is this input picking anything up?" for every listed device.
//!
//! One tiny peak-detect stream per unmuted source (25 peak values per second,
//! the same technique pavucontrol uses for its level bars). Muted sources are
//! not monitored: the server mutes them before we would see any signal.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

use libpulse_binding as pa;
use libpulse_binding::context::Context;
use libpulse_binding::stream::{FlagSet, PeekResult, State, Stream};

use crate::pulse::SourceEntry;

/// Linear peak above which a source counts as picking up sound (about -45 dBFS).
const ACTIVE_PEAK: f32 = 0.0056;
/// Keep showing a source as active this long after the last loud peak.
const HOLD: Duration = Duration::from_millis(300);
const PEAK_RATE: u32 = 25;

struct Monitor {
    stream: Rc<RefCell<Stream>>,
    last_active: Rc<Cell<Option<Instant>>>,
    failed: Rc<Cell<bool>>,
}

impl Drop for Monitor {
    fn drop(&mut self) {
        let mut s = self.stream.borrow_mut();
        s.set_state_callback(None);
        s.set_read_callback(None);
        if s.get_state().is_good() {
            let _ = s.disconnect();
        }
    }
}

#[derive(Default)]
pub struct ActivityMonitor {
    monitors: HashMap<String, Monitor>,
}

impl ActivityMonitor {
    /// Start / stop monitors so exactly the unmuted `sources` are watched.
    pub fn sync(&mut self, ctx: &Rc<RefCell<Context>>, sources: &[SourceEntry]) {
        self.monitors.retain(|name, m| {
            !m.failed.get() && sources.iter().any(|s| &s.name == name && !s.muted)
        });
        for s in sources.iter().filter(|s| !s.muted) {
            if !self.monitors.contains_key(&s.name) {
                if let Some(m) = start(ctx, &s.name) {
                    self.monitors.insert(s.name.clone(), m);
                }
            }
        }
    }

    pub fn clear(&mut self) {
        self.monitors.clear();
    }

    pub fn is_active(&self, source: &str) -> bool {
        self.monitors
            .get(source)
            .and_then(|m| m.last_active.get())
            .is_some_and(|t| t.elapsed() < HOLD)
    }
}

fn start(ctx: &Rc<RefCell<Context>>, source: &str) -> Option<Monitor> {
    let spec = pa::sample::Spec {
        format: pa::sample::Format::F32le,
        rate: PEAK_RATE,
        channels: 1,
    };
    let stream = Stream::new(&mut ctx.borrow_mut(), "Sound Checker activity", &spec, None)?;
    let stream = Rc::new(RefCell::new(stream));
    let last_active = Rc::new(Cell::new(None));
    let failed = Rc::new(Cell::new(false));

    {
        let st = Rc::downgrade(&stream);
        let last = Rc::clone(&last_active);
        stream.borrow_mut().set_read_callback(Some(Box::new(move |_| {
            let Some(st) = st.upgrade() else { return };
            // Can run inside connect_record while the stream is borrowed.
            let Ok(mut st) = st.try_borrow_mut() else { return };
            loop {
                match st.peek() {
                    Ok(PeekResult::Data(data)) => {
                        let peak = data
                            .chunks_exact(4)
                            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]).abs())
                            .fold(0.0, f32::max);
                        if peak > ACTIVE_PEAK {
                            last.set(Some(Instant::now()));
                        }
                        let _ = st.discard();
                    }
                    Ok(PeekResult::Hole(_)) => {
                        let _ = st.discard();
                    }
                    Ok(PeekResult::Empty) | Err(_) => break,
                }
            }
        })));
    }
    {
        let st = Rc::downgrade(&stream);
        let failed = Rc::clone(&failed);
        stream.borrow_mut().set_state_callback(Some(Box::new(move || {
            let Some(st) = st.upgrade() else { return };
            let Ok(st) = st.try_borrow() else { return };
            if matches!(st.get_state(), State::Failed | State::Terminated) {
                failed.set(true);
            }
        })));
    }

    let attr = pa::def::BufferAttr {
        maxlength: u32::MAX,
        tlength: u32::MAX,
        prebuf: u32::MAX,
        minreq: u32::MAX,
        fragsize: 4,
    };
    stream
        .borrow_mut()
        .connect_record(
            Some(source),
            Some(&attr),
            FlagSet::PEAK_DETECT | FlagSet::ADJUST_LATENCY | FlagSet::DONT_MOVE,
        )
        .ok()?;
    Some(Monitor { stream, last_active, failed })
}
