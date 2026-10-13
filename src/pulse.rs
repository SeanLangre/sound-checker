//! PulseAudio client (works against `pipewire-pulse` too), driven by the GLib
//! main loop so every callback runs on the GTK thread.
//!
//! The app only ever connects to an existing server (`NOAUTOSPAWN`).

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;

use gtk::glib;
use libpulse_binding as pa;
use libpulse_binding::context::introspect::{CardInfo, ServerInfo, SinkInfo, SourceInfo};
use libpulse_binding::context::subscribe::InterestMaskSet;
use libpulse_binding::context::{Context, FlagSet, State};
use libpulse_binding::callbacks::ListResult;
use libpulse_binding::volume::{ChannelVolumes, Volume};
use libpulse_glib_binding::Mainloop;

/// Coalesce bursts of server events (a hot-plug emits several) into one refresh.
const REFRESH_DEBOUNCE: Duration = Duration::from_millis(150);

#[derive(Debug, Clone)]
pub struct SourceEntry {
    pub name: String,
    /// PipeWire `node.name` (what WirePlumber rules match on).
    pub node_name: String,
    pub device_name: Option<String>,
    pub description: String,
    pub muted: bool,
    pub volume_pct: u32,
    pub volumes: ChannelVolumes,
    pub rate: u32,
    pub channels: u8,
    pub format: String,
    pub channel_map: String,
    pub driver: String,
    pub card: Option<u32>,
    pub bus: Option<String>,
    pub form_factor: Option<String>,
    pub state: String,
    pub is_default: bool,
}

/// An output (playback device).
#[derive(Debug, Clone)]
pub struct SinkEntry {
    pub name: String,
    /// PipeWire `node.name` (what WirePlumber rules match on).
    pub node_name: String,
    pub device_name: Option<String>,
    pub description: String,
    pub muted: bool,
    pub volume_pct: u32,
    pub volumes: ChannelVolumes,
    pub rate: u32,
    pub channels: u8,
    pub format: String,
    pub channel_map: String,
    pub driver: String,
    pub card: Option<u32>,
    pub bus: Option<String>,
    pub form_factor: Option<String>,
    pub state: String,
    pub is_default: bool,
}

#[derive(Debug, Clone)]
pub struct ProfileEntry {
    pub name: String,
    pub available: bool,
}

#[derive(Debug, Clone)]
pub struct CardEntry {
    pub index: u32,
    pub name: String,
    pub device_name: Option<String>,
    pub description: String,
    pub bus: Option<String>,
    pub active_profile: Option<String>,
    pub profiles: Vec<ProfileEntry>,
}

impl CardEntry {
    pub fn is_bluetooth(&self) -> bool {
        self.bus.as_deref() == Some("bluetooth") || self.name.starts_with("bluez_card")
    }

    /// For a Bluetooth card in a headset profile: the output-only A2DP profile
    /// that drops its microphone.
    pub fn music_profile_hint(&self) -> Option<&ProfileEntry> {
        if !self.is_bluetooth() || self.active_profile.as_deref()?.starts_with("a2dp") {
            return None;
        }
        self.profiles
            .iter()
            .find(|p| p.available && p.name.starts_with("a2dp-sink"))
    }

    /// For a Bluetooth card sitting in an A2DP (output-only) profile that also
    /// offers a headset (HFP/HSP) profile: the profile to switch to for a mic.
    pub fn headset_profile_hint(&self) -> Option<&ProfileEntry> {
        if !self.is_bluetooth() {
            return None;
        }
        let active = self.active_profile.as_deref()?;
        if !active.starts_with("a2dp") {
            return None;
        }
        self.profiles
            .iter()
            .filter(|p| p.available && p.name.starts_with("headset-head-unit"))
            // Prefer the wideband (mSBC) variant when offered.
            .max_by_key(|p| p.name.contains("msbc"))
    }
}

#[derive(Debug, Clone, Default)]
pub struct Model {
    pub sources: Vec<SourceEntry>,
    pub sinks: Vec<SinkEntry>,
    /// Monitor sources: what each output is playing (name `<sink>.monitor`).
    pub monitors: Vec<SourceEntry>,
    pub cards: Vec<CardEntry>,
    pub server_name: String,
}

impl Model {
    pub fn is_pipewire(&self) -> bool {
        self.server_name.contains("PipeWire")
    }

    /// Bluetooth cards in A2DP with no capture source of their own, i.e. the
    /// headsets that look like they have "no mic".
    pub fn bluetooth_without_mic(&self) -> impl Iterator<Item = &CardEntry> {
        self.cards.iter().filter(move |c| {
            c.headset_profile_hint().is_some()
                && !self.sources.iter().any(|s| s.card == Some(c.index))
        })
    }
}

pub enum Event {
    Update(Model),
    Disconnected(String),
}

pub struct Backend {
    pub ctx: Rc<RefCell<Context>>,
    _mainloop: Mainloop,
    on_event: Box<dyn Fn(Event)>,
    refreshing: Cell<bool>,
    dirty: Cell<bool>,
    timer_pending: Cell<bool>,
    dead: Cell<bool>,
}

impl Backend {
    pub fn new(on_event: impl Fn(Event) + 'static) -> Result<Rc<Backend>, String> {
        let mainloop = Mainloop::new(None).ok_or("could not create the PulseAudio main loop")?;
        let mut ctx = Context::new(&mainloop, "Linux Sound Checker")
            .ok_or("could not create a PulseAudio context")?;
        ctx.connect(None, FlagSet::NOAUTOSPAWN, None)
            .map_err(|e| format!("could not connect to the audio server: {e}"))?;

        let backend = Rc::new(Backend {
            ctx: Rc::new(RefCell::new(ctx)),
            _mainloop: mainloop,
            on_event: Box::new(on_event),
            refreshing: Cell::new(false),
            dirty: Cell::new(false),
            timer_pending: Cell::new(false),
            dead: Cell::new(false),
        });

        let weak = Rc::downgrade(&backend);
        backend
            .ctx
            .borrow_mut()
            .set_state_callback(Some(Box::new(move || {
                if let Some(b) = weak.upgrade() {
                    b.on_state_change();
                }
            })));
        Ok(backend)
    }

    fn on_state_change(self: &Rc<Self>) {
        let state = self.ctx.borrow().get_state();
        match state {
            State::Ready => {
                let weak = Rc::downgrade(self);
                let mut ctx = self.ctx.borrow_mut();
                ctx.set_subscribe_callback(Some(Box::new(move |_facility, _op, _idx| {
                    if let Some(b) = weak.upgrade() {
                        b.schedule_refresh();
                    }
                })));
                ctx.subscribe(
                    InterestMaskSet::SOURCE | InterestMaskSet::SINK | InterestMaskSet::CARD | InterestMaskSet::SERVER,
                    |_ok| {},
                );
                drop(ctx);
                self.schedule_refresh();
            }
            State::Failed | State::Terminated => {
                if !self.dead.replace(true) {
                    (self.on_event)(Event::Disconnected(
                        "The audio server went away or refused the connection.".into(),
                    ));
                }
            }
            _ => {}
        }
    }

    pub fn schedule_refresh(self: &Rc<Self>) {
        if self.timer_pending.replace(true) {
            return;
        }
        let weak: Weak<Backend> = Rc::downgrade(self);
        glib::timeout_add_local_once(REFRESH_DEBOUNCE, move || {
            if let Some(b) = weak.upgrade() {
                b.timer_pending.set(false);
                b.refresh();
            }
        });
    }

    /// Re-read sources, cards and the default source, then emit `Event::Update`.
    fn refresh(self: &Rc<Self>) {
        if self.dead.get() || self.ctx.borrow().get_state() != State::Ready {
            return;
        }
        if self.refreshing.replace(true) {
            // A refresh is in flight; run another when it finishes.
            self.dirty.set(true);
            return;
        }

        let model = Rc::new(RefCell::new(Model::default()));
        let default_source: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));

        let introspect = self.ctx.borrow().introspect();

        // Chain: sources -> cards -> server info -> emit.
        let this = Rc::clone(self);
        let model_s = Rc::clone(&model);
        let default_s = Rc::clone(&default_source);
        introspect.get_source_info_list(move |res| match res {
            ListResult::Item(info) => {
                // Monitor sources are loopbacks of outputs, not inputs.
                let entry = source_entry(info);
                if info.monitor_of_sink.is_none() {
                    model_s.borrow_mut().sources.push(entry);
                } else {
                    model_s.borrow_mut().monitors.push(entry);
                }
            }
            ListResult::Error => this.finish_refresh(None),
            ListResult::End => {
                let this_k = Rc::clone(&this);
                let model_k = Rc::clone(&model_s);
                let default_k = Rc::clone(&default_s);
                this.ctx.borrow().introspect().get_sink_info_list(move |res| match res {
                    ListResult::Item(info) => model_k.borrow_mut().sinks.push(sink_entry(info)),
                    ListResult::Error => this_k.finish_refresh(None),
                    ListResult::End => {
                let this2 = Rc::clone(&this_k);
                let model_c = Rc::clone(&model_k);
                let default_c = Rc::clone(&default_k);
                let introspect = this_k.ctx.borrow().introspect();
                introspect.get_card_info_list(move |res| match res {
                    ListResult::Item(info) => model_c.borrow_mut().cards.push(card_entry(info)),
                    ListResult::Error => this2.finish_refresh(None),
                    ListResult::End => {
                        let this3 = Rc::clone(&this2);
                        let model_d = Rc::clone(&model_c);
                        let default_d = Rc::clone(&default_c);
                        this2
                            .ctx
                            .borrow()
                            .introspect()
                            .get_server_info(move |info: &ServerInfo| {
                                *default_d.borrow_mut() =
                                    info.default_source_name.as_ref().map(|s| s.to_string());
                                let mut m = model_d.borrow().clone();
                                m.server_name =
                                    info.server_name.as_deref().unwrap_or_default().to_string();
                                for s in &mut m.sources {
                                    s.is_default =
                                        default_d.borrow().as_deref() == Some(s.name.as_str());
                                }
                                let default_sink =
                                    info.default_sink_name.as_ref().map(|s| s.to_string());
                                for s in &mut m.sinks {
                                    s.is_default = default_sink.as_deref() == Some(s.name.as_str());
                                }
                                this3.finish_refresh(Some(m));
                            });
                    }
                });
                    }
                });
            }
        });
    }

    fn finish_refresh(self: &Rc<Self>, model: Option<Model>) {
        self.refreshing.set(false);
        if let Some(model) = model {
            (self.on_event)(Event::Update(model));
        }
        if self.dirty.replace(false) {
            self.schedule_refresh();
        }
    }

    pub fn set_mute(self: &Rc<Self>, source: &str, mute: bool) {
        let this = Rc::clone(self);
        self.ctx
            .borrow()
            .introspect()
            .set_source_mute_by_name(source, mute, Some(Box::new(move |_ok| this.schedule_refresh())));
    }

    /// Set every channel of `source` to 100 %.
    pub fn reset_volume(self: &Rc<Self>, source: &SourceEntry) {
        let mut vols = source.volumes;
        vols.set(source.channels, Volume::NORMAL);
        let this = Rc::clone(self);
        self.ctx.borrow().introspect().set_source_volume_by_name(
            &source.name,
            &vols,
            Some(Box::new(move |_ok| this.schedule_refresh())),
        );
    }

    pub fn set_sink_mute(self: &Rc<Self>, sink: &str, mute: bool) {
        let this = Rc::clone(self);
        self.ctx
            .borrow()
            .introspect()
            .set_sink_mute_by_name(sink, mute, Some(Box::new(move |_ok| this.schedule_refresh())));
    }

    pub fn reset_sink_volume(self: &Rc<Self>, sink: &SinkEntry) {
        let mut vols = sink.volumes;
        vols.set(sink.channels, Volume::NORMAL);
        let this = Rc::clone(self);
        self.ctx.borrow().introspect().set_sink_volume_by_name(
            &sink.name,
            &vols,
            Some(Box::new(move |_ok| this.schedule_refresh())),
        );
    }

    pub fn set_default_source(self: &Rc<Self>, name: &str) {
        let this = Rc::clone(self);
        self.ctx
            .borrow_mut()
            .set_default_source(name, move |_ok| this.schedule_refresh());
    }

    pub fn set_default_sink(self: &Rc<Self>, name: &str) {
        let this = Rc::clone(self);
        self.ctx
            .borrow_mut()
            .set_default_sink(name, move |_ok| this.schedule_refresh());
    }

    pub fn set_card_profile(self: &Rc<Self>, card: &str, profile: &str) {
        let this = Rc::clone(self);
        self.ctx.borrow().introspect().set_card_profile_by_name(
            card,
            profile,
            Some(Box::new(move |_ok| this.schedule_refresh())),
        );
    }
}

fn source_entry(info: &SourceInfo) -> SourceEntry {
    let prop = |k: &str| info.proplist.get_str(k);
    let name = info.name.as_deref().unwrap_or_default().to_string();
    SourceEntry {
        node_name: prop("node.name").unwrap_or_else(|| name.clone()),
        device_name: prop("device.name"),
        name,
        description: info
            .description
            .as_deref()
            .or(info.name.as_deref())
            .unwrap_or("Unknown device")
            .to_string(),
        muted: info.mute,
        volume_pct: volume_pct(info.volume.avg()),
        volumes: info.volume,
        rate: info.sample_spec.rate,
        channels: info.sample_spec.channels,
        format: format!("{:?}", info.sample_spec.format),
        channel_map: info.channel_map.print(),
        driver: info.driver.as_deref().unwrap_or("unknown").to_string(),
        card: info.card,
        bus: prop("device.bus"),
        form_factor: prop("device.form_factor"),
        state: format!("{:?}", info.state),
        is_default: false,
    }
}

fn sink_entry(info: &SinkInfo) -> SinkEntry {
    let prop = |k: &str| info.proplist.get_str(k);
    let name = info.name.as_deref().unwrap_or_default().to_string();
    SinkEntry {
        node_name: prop("node.name").unwrap_or_else(|| name.clone()),
        device_name: prop("device.name"),
        description: info
            .description
            .as_deref()
            .or(info.name.as_deref())
            .unwrap_or("Unknown device")
            .to_string(),
        name,
        muted: info.mute,
        volume_pct: volume_pct(info.volume.avg()),
        volumes: info.volume,
        rate: info.sample_spec.rate,
        channels: info.sample_spec.channels,
        format: format!("{:?}", info.sample_spec.format),
        channel_map: info.channel_map.print(),
        driver: info.driver.as_deref().unwrap_or("unknown").to_string(),
        card: info.card,
        bus: prop("device.bus"),
        form_factor: prop("device.form_factor"),
        state: format!("{:?}", info.state),
        is_default: false,
    }
}

fn card_entry(info: &CardInfo) -> CardEntry {
    CardEntry {
        index: info.index,
        name: info.name.as_deref().unwrap_or_default().to_string(),
        device_name: info.proplist.get_str("device.name"),
        description: info
            .proplist
            .get_str("device.description")
            .or_else(|| info.name.as_deref().map(str::to_string))
            .unwrap_or_else(|| "Unknown card".into()),
        bus: info.proplist.get_str("device.bus"),
        active_profile: info
            .active_profile
            .as_ref()
            .and_then(|p| p.name.as_deref())
            .map(str::to_string),
        profiles: info
            .profiles
            .iter()
            .map(|p| ProfileEntry {
                name: p.name.as_deref().unwrap_or_default().to_string(),
                available: p.available,
            })
            .collect(),
    }
}

fn volume_pct(v: Volume) -> u32 {
    ((v.0 as f64 / Volume::NORMAL.0 as f64) * 100.0).round() as u32
}

/// Sample spec for capturing from `source`: float32, the source's own rate and
/// channel count so nothing is resampled or remixed behind our back.
pub fn capture_spec(source: &SourceEntry) -> pa::sample::Spec {
    pa::sample::Spec {
        format: pa::sample::Format::F32le,
        rate: source.rate,
        channels: source.channels,
    }
}
