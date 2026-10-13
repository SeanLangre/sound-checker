//! GTK4 + libadwaita user interface.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;

use crate::analysis::{ChannelLevel, Status, FLOOR_DB};
use crate::capture::{Capture, CaptureState, RecordState, RECORD_SECS};
use crate::pulse::{Backend, CardEntry, Event, Model, SourceEntry};

const METER_MIN_DB: f32 = -60.0;
const METER_ROW_H: i32 = 28;
const AMBER_DB: f32 = -12.0;
const RED_DB: f32 = -3.0;

#[derive(Clone)]
enum RowKind {
    Source(String),
    BluetoothCard,
}

#[derive(Default)]
struct MeterData {
    names: Vec<String>,
    levels: Vec<ChannelLevel>,
}

struct State {
    model: Model,
    rows: Vec<RowKind>,
    selected: Option<String>,
    /// What the running capture was started with, to detect spec changes.
    capture_spec: Option<(String, u32, u8)>,
    capture: Option<Capture>,
    backend: Option<Rc<Backend>>,
}

struct Ui {
    window: adw::ApplicationWindow,
    stack: gtk::Stack,
    error_page: adw::StatusPage,
    list: gtk::ListBox,
    rebuilding: Cell<bool>,

    // Right-hand pane.
    placeholder: gtk::Label,
    analysis: gtk::Box,
    title: gtk::Label,
    warn_row: gtk::Box,
    warn_label: gtk::Label,
    unmute_btn: gtk::Button,
    volume_btn: gtk::Button,
    meters: gtk::DrawingArea,
    wave: gtk::DrawingArea,
    stats: gtk::Label,
    status: gtk::Label,
    details: gtk::Label,
    record_btn: gtk::Button,
    play_btn: gtk::Button,
    record_label: gtk::Label,

    meter_data: Rc<RefCell<MeterData>>,
    wave_data: Rc<RefCell<Vec<(f32, f32)>>>,
    state: RefCell<State>,
}

pub fn build(app: &adw::Application) {
    let ui = Ui::new(app);
    ui.connect();
    ui.window.present();
    // Every callback holds only a weak reference, so give the window the one
    // strong one; it is released when the window is destroyed.
    let keep_alive = Rc::clone(&ui);
    ui.window.connect_destroy(move |_| {
        let _ = &keep_alive;
    });
}

fn label(text: &str) -> gtk::Label {
    gtk::Label::builder().label(text).xalign(0.0).wrap(true).build()
}

impl Ui {
    fn new(app: &adw::Application) -> Rc<Ui> {
        // Left: device list.
        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::Single)
            .css_classes(["navigation-sidebar"])
            .build();
        let list_scroll = gtk::ScrolledWindow::builder()
            .child(&list)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .min_content_width(280)
            .vexpand(true)
            .build();

        // Right: analysis.
        let title = label("");
        title.add_css_class("title-2");

        let warn_label = label("");
        warn_label.set_hexpand(true);
        let unmute_btn = gtk::Button::with_label("Unmute");
        let volume_btn = gtk::Button::with_label("Set volume to 100%");
        let warn_row = gtk::Box::builder()
            .spacing(8)
            .css_classes(["card"])
            .margin_top(0)
            .build();
        for w in [warn_label.upcast_ref::<gtk::Widget>(), unmute_btn.upcast_ref(), volume_btn.upcast_ref()] {
            w.set_margin_top(8);
            w.set_margin_bottom(8);
            warn_row.append(w);
        }
        warn_label.set_margin_start(12);
        volume_btn.set_margin_end(8);
        warn_row.set_visible(false);

        let meters = gtk::DrawingArea::builder().hexpand(true).content_height(METER_ROW_H * 2).build();
        let wave = gtk::DrawingArea::builder().hexpand(true).content_height(120).build();
        wave.add_css_class("card");

        let stats = label("");
        stats.add_css_class("monospace");
        let status = label("");
        status.add_css_class("heading");

        let record_btn = gtk::Button::with_label(&format!("● Record {RECORD_SECS} s"));
        let play_btn = gtk::Button::with_label("▶ Play back");
        play_btn.set_sensitive(false);
        let record_label = label("");
        record_label.add_css_class("dim-label");
        let rec_row = gtk::Box::builder().spacing(8).build();
        rec_row.append(&record_btn);
        rec_row.append(&play_btn);
        rec_row.append(&record_label);

        let details = label("");
        details.set_selectable(true);
        details.add_css_class("monospace");
        let expander = gtk::Expander::builder().label("Device details").child(&details).build();

        let analysis = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(12)
            .margin_top(16)
            .margin_bottom(16)
            .margin_start(16)
            .margin_end(16)
            .hexpand(true)
            .build();
        for w in [
            title.upcast_ref::<gtk::Widget>(),
            warn_row.upcast_ref(),
            meters.upcast_ref(),
            stats.upcast_ref(),
            wave.upcast_ref(),
            status.upcast_ref(),
            rec_row.upcast_ref(),
            expander.upcast_ref(),
        ] {
            analysis.append(w);
        }
        analysis.set_visible(false);

        let placeholder = label("Select an input device to test it.");
        placeholder.set_halign(gtk::Align::Center);
        placeholder.set_valign(gtk::Align::Center);
        placeholder.set_hexpand(true);
        placeholder.add_css_class("dim-label");

        let right = gtk::Box::new(gtk::Orientation::Vertical, 0);
        right.append(&placeholder);
        right.append(&analysis);
        let right_scroll = gtk::ScrolledWindow::builder()
            .child(&right)
            .hexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build();

        let paned = gtk::Paned::builder()
            .start_child(&list_scroll)
            .end_child(&right_scroll)
            .shrink_start_child(false)
            .position(320)
            .build();

        let error_page = adw::StatusPage::builder()
            .icon_name("audio-input-microphone-symbolic")
            .title("Can't reach the audio server")
            .build();
        let retry = gtk::Button::builder()
            .label("Retry")
            .halign(gtk::Align::Center)
            .css_classes(["suggested-action", "pill"])
            .build();
        error_page.set_child(Some(&retry));

        let stack = gtk::Stack::new();
        stack.add_named(&paned, Some("main"));
        stack.add_named(&error_page, Some("error"));

        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&adw::HeaderBar::new());
        toolbar.set_content(Some(&stack));

        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title("Linux Sound Checker")
            .default_width(960)
            .default_height(640)
            .content(&toolbar)
            .build();

        let ui = Rc::new(Ui {
            window,
            stack,
            error_page,
            list,
            rebuilding: Cell::new(false),
            placeholder,
            analysis,
            title,
            warn_row,
            warn_label,
            unmute_btn,
            volume_btn,
            meters,
            wave,
            stats,
            status,
            details,
            record_btn,
            play_btn,
            record_label,
            meter_data: Rc::new(RefCell::new(MeterData::default())),
            wave_data: Rc::new(RefCell::new(Vec::new())),
            state: RefCell::new(State {
                model: Model::default(),
                rows: Vec::new(),
                selected: None,
                capture_spec: None,
                capture: None,
                backend: None,
            }),
        });

        let weak = Rc::downgrade(&ui);
        retry.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                ui.connect();
            }
        });
        ui
    }

    /// (Re)connect to the audio server and wire up all handlers.
    fn connect(self: &Rc<Self>) {
        {
            let mut st = self.state.borrow_mut();
            st.capture = None;
            st.capture_spec = None;
            st.backend = None;
        }
        let weak = Rc::downgrade(self);
        let backend = Backend::new(move |ev| {
            if let Some(ui) = weak.upgrade() {
                ui.on_event(ev);
            }
        });
        match backend {
            Ok(b) => {
                self.state.borrow_mut().backend = Some(b);
                self.stack.set_visible_child_name("main");
                self.wire_once();
            }
            Err(msg) => self.show_error(&msg),
        }
    }

    fn show_error(&self, msg: &str) {
        self.error_page.set_description(Some(msg));
        self.stack.set_visible_child_name("error");
    }

    /// Hook up widget signals; guarded so reconnecting doesn't duplicate them.
    fn wire_once(self: &Rc<Self>) {
        thread_local!(static WIRED: Cell<bool> = const { Cell::new(false) });
        if WIRED.with(|w| w.replace(true)) {
            return;
        }

        let weak = Rc::downgrade(self);
        let w = weak.clone();
        self.list.connect_row_selected(move |_, row| {
            let (Some(ui), Some(row)) = (w.upgrade(), row) else { return };
            if ui.rebuilding.get() {
                return;
            }
            ui.on_row_selected(row.index() as usize);
        });

        let w = weak.clone();
        self.unmute_btn.connect_clicked(move |_| {
            if let Some(ui) = w.upgrade() {
                if let (Some(b), Some(s)) = (ui.backend(), ui.selected_source()) {
                    b.set_mute(&s.name, false);
                }
            }
        });
        let w = weak.clone();
        self.volume_btn.connect_clicked(move |_| {
            if let Some(ui) = w.upgrade() {
                if let (Some(b), Some(s)) = (ui.backend(), ui.selected_source()) {
                    b.reset_volume(&s);
                }
            }
        });
        let w = weak.clone();
        self.record_btn.connect_clicked(move |_| {
            if let Some(ui) = w.upgrade() {
                if let Some(c) = &ui.state.borrow().capture {
                    c.start_recording();
                }
            }
        });
        let w = weak.clone();
        self.play_btn.connect_clicked(move |_| {
            if let Some(ui) = w.upgrade() {
                if let Some(c) = &ui.state.borrow().capture {
                    if c.shared.borrow().record == RecordState::Playing {
                        c.stop_playback();
                    } else {
                        c.start_playback();
                    }
                }
            }
        });

        // Drawing.
        let data = Rc::clone(&self.meter_data);
        self.meters.set_draw_func(move |area, cr, w, h| draw_meters(area, cr, w, h, &data.borrow()));
        let data = Rc::clone(&self.wave_data);
        self.wave.set_draw_func(move |area, cr, w, h| draw_wave(area, cr, w, h, &data.borrow()));

        // Frame-synced refresh from the latest analysis snapshot.
        let w = weak.clone();
        self.analysis.add_tick_callback(move |_, _| {
            match w.upgrade() {
                Some(ui) => {
                    ui.tick();
                    glib::ControlFlow::Continue
                }
                None => glib::ControlFlow::Break,
            }
        });
    }

    fn backend(&self) -> Option<Rc<Backend>> {
        self.state.borrow().backend.clone()
    }

    fn selected_source(&self) -> Option<SourceEntry> {
        let st = self.state.borrow();
        let name = st.selected.as_ref()?;
        st.model.sources.iter().find(|s| &s.name == name).cloned()
    }

    fn on_event(self: &Rc<Self>, ev: Event) {
        match ev {
            Event::Disconnected(msg) => {
                {
                    let mut st = self.state.borrow_mut();
                    st.capture = None;
                    st.capture_spec = None;
                }
                self.show_error(&msg);
            }
            Event::Update(model) => {
                self.state.borrow_mut().model = model;
                self.rebuild_list();
                self.sync_selection();
            }
        }
    }

    fn rebuild_list(self: &Rc<Self>) {
        self.rebuilding.set(true);
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        let mut rows = Vec::new();
        {
            let st = self.state.borrow();
            for s in &st.model.sources {
                let row = adw::ActionRow::builder()
                    .title(glib::markup_escape_text(&s.description))
                    .subtitle(glib::markup_escape_text(&source_subtitle(s)))
                    .build();
                row.add_prefix(&gtk::Image::from_icon_name("audio-input-microphone-symbolic"));
                if s.muted {
                    let img = gtk::Image::from_icon_name("microphone-sensitivity-muted-symbolic");
                    img.set_tooltip_text(Some("Muted"));
                    row.add_suffix(&img);
                }
                if s.is_default {
                    let tag = gtk::Label::new(Some("default"));
                    tag.add_css_class("dim-label");
                    row.add_suffix(&tag);
                }
                self.list.append(&row);
                rows.push(RowKind::Source(s.name.clone()));
            }
            for c in st.model.bluetooth_without_mic() {
                let row = adw::ActionRow::builder()
                    .title(glib::markup_escape_text(&c.description))
                    .subtitle("No microphone in A2DP (music) mode")
                    .build();
                row.add_prefix(&gtk::Image::from_icon_name("bluetooth-symbolic"));
                let btn = gtk::Button::builder()
                    .label("Switch to headset mode")
                    .valign(gtk::Align::Center)
                    .build();
                let weak = Rc::downgrade(self);
                let card = c.name.clone();
                btn.connect_clicked(move |_| {
                    if let Some(ui) = weak.upgrade() {
                        ui.switch_to_headset(&card);
                    }
                });
                row.add_suffix(&btn);
                row.set_activatable(false);
                self.list.append(&row);
                rows.push(RowKind::BluetoothCard);
            }
        }
        self.state.borrow_mut().rows = rows;
        self.rebuilding.set(false);
    }

    fn switch_to_headset(&self, card_name: &str) {
        let (backend, profile) = {
            let st = self.state.borrow();
            let profile = st
                .model
                .cards
                .iter()
                .find(|c| c.name == card_name)
                .and_then(CardEntry::headset_profile_hint)
                .map(|p| p.name.clone());
            (st.backend.clone(), profile)
        };
        if let (Some(b), Some(p)) = (backend, profile) {
            b.set_card_profile(card_name, &p);
        }
    }

    fn on_row_selected(self: &Rc<Self>, index: usize) {
        let kind = self.state.borrow().rows.get(index).cloned();
        if let Some(RowKind::Source(name)) = kind {
            self.state.borrow_mut().selected = Some(name);
            self.sync_selection();
        }
    }

    /// Reconcile the selection / capture with the latest model.
    fn sync_selection(self: &Rc<Self>) {
        // Pick the default source when nothing is selected, or the selection vanished.
        let (needs_pick, gone) = {
            let st = self.state.borrow();
            let present = st
                .selected
                .as_ref()
                .is_some_and(|n| st.model.sources.iter().any(|s| &s.name == n));
            (!present, st.selected.is_some() && !present)
        };
        if gone {
            let mut st = self.state.borrow_mut();
            st.capture = None;
            st.capture_spec = None;
            st.selected = None;
        }
        if needs_pick {
            let pick = {
                let st = self.state.borrow();
                st.model
                    .sources
                    .iter()
                    .find(|s| s.is_default)
                    .or(st.model.sources.first())
                    .map(|s| s.name.clone())
            };
            self.state.borrow_mut().selected = pick;
        }

        let source = self.selected_source();
        self.placeholder.set_visible(source.is_none());
        self.analysis.set_visible(source.is_some());
        if gone {
            self.placeholder.set_label("The selected device was disconnected.");
        }
        let Some(source) = source else { return };

        // Reflect selection in the list without re-triggering.
        self.rebuilding.set(true);
        let idx = self
            .state
            .borrow()
            .rows
            .iter()
            .position(|r| matches!(r, RowKind::Source(n) if *n == source.name));
        if let Some(row) = idx.and_then(|i| self.list.row_at_index(i as i32)) {
            self.list.select_row(Some(&row));
        }
        self.rebuilding.set(false);

        self.update_source_info(&source);

        // (Re)start capture when the device or its format changed.
        let spec = (source.name.clone(), source.rate, source.channels);
        let restart = self.state.borrow().capture_spec.as_ref() != Some(&spec);
        if restart {
            self.start_capture(&source);
        }
    }

    fn start_capture(&self, source: &SourceEntry) {
        let backend = self.backend();
        let mut st = self.state.borrow_mut();
        st.capture = None;
        st.capture_spec = Some((source.name.clone(), source.rate, source.channels));
        let names = channel_names(source);
        *self.meter_data.borrow_mut() = MeterData {
            levels: vec![
                ChannelLevel { peak_db: FLOOR_DB, rms_db: FLOOR_DB, held_peak_db: FLOOR_DB };
                names.len()
            ],
            names,
        };
        self.meters
            .set_content_height(METER_ROW_H * source.channels.max(1) as i32);
        self.wave_data.borrow_mut().clear();
        self.play_btn.set_sensitive(false);
        self.record_label.set_text("");

        let Some(backend) = backend else { return };
        match Capture::start(&backend.ctx, source) {
            Ok(c) => st.capture = Some(c),
            Err(e) => {
                drop(st);
                self.status.set_text(&format!("Capture error: {e}"));
                set_status_class(&self.status, "error");
            }
        }
    }

    fn update_source_info(&self, s: &SourceEntry) {
        self.title.set_text(&s.description);

        let mut problems = Vec::new();
        if s.muted {
            problems.push("This input is muted.".to_string());
        } else if s.volume_pct == 0 {
            problems.push("Input volume is 0%.".to_string());
        } else if s.volume_pct < 100 {
            problems.push(format!("Input volume is {}%.", s.volume_pct));
        }
        self.warn_label.set_text(&problems.join(" "));
        self.unmute_btn.set_visible(s.muted);
        self.volume_btn.set_visible(s.volume_pct != 100);
        self.warn_row.set_visible(s.muted || s.volume_pct != 100);

        let st = self.state.borrow();
        let card = s
            .card
            .and_then(|i| st.model.cards.iter().find(|c| c.index == i))
            .map(|c| {
                format!(
                    "{} (profile: {})",
                    c.description,
                    c.active_profile.as_deref().unwrap_or("none")
                )
            })
            .unwrap_or_else(|| "none".into());
        let details = format!(
            "Name:        {}\nSample rate: {} Hz\nFormat:      {}\nChannels:    {} ({})\nDriver:      {}\nBus:         {}\nForm factor: {}\nState:       {}\nVolume:      {}%{}\nCard:        {}",
            s.name,
            s.rate,
            s.format,
            s.channels,
            s.channel_map,
            s.driver,
            s.bus.as_deref().unwrap_or("unknown"),
            s.form_factor.as_deref().unwrap_or("unknown"),
            s.state,
            s.volume_pct,
            if s.muted { " (muted)" } else { "" },
            card,
        );
        self.details.set_text(&details);
    }

    /// Per-frame: pull the latest analysis and redraw.
    fn tick(&self) {
        let st = self.state.borrow();
        // With no capture (failed to start) the last status message stays up.
        let Some(capture) = &st.capture else { return };
        let sh = capture.shared.borrow();

        let snap = sh.analyzer.snapshot();
        {
            let mut m = self.meter_data.borrow_mut();
            m.levels = snap.channels.clone();
        }
        {
            let mut w = self.wave_data.borrow_mut();
            w.clear();
            w.extend(sh.analyzer.waveform());
        }
        self.meters.queue_draw();
        self.wave.queue_draw();

        let noise = snap
            .noise_floor_db
            .map(|n| format!("{n:.0} dBFS"))
            .unwrap_or_else(|| "–".into());
        self.stats.set_text(&format!(
            "RMS {:.1} dBFS    Noise floor {noise}",
            snap.rms_db
        ));

        let (text, class) = match &sh.state {
            CaptureState::Connecting => ("Connecting to the device…".to_string(), "dim-label"),
            CaptureState::Failed(why) => (
                format!("Capture stopped: {why}. Re-select the device to retry."),
                "error",
            ),
            CaptureState::Running => match snap.status {
                Status::Waiting => ("Waiting for audio…".into(), "dim-label"),
                Status::Ok => ("OK — signal looks healthy".into(), "success"),
                Status::Silence => (
                    "No signal — check that the mic is unmuted, plugged in and is the device you meant to select".into(),
                    "warning",
                ),
                Status::Clipping => ("Clipping — turn the input volume down".into(), "error"),
                Status::DeadChannel(ch) => {
                    let names = &self.meter_data.borrow().names;
                    let n = names.get(ch).cloned().unwrap_or_else(|| format!("{}", ch + 1));
                    (format!("Channel {n} is silent while the others carry signal"), "warning")
                }
                Status::Imbalanced => ("Channels are badly imbalanced (>12 dB apart)".into(), "warning"),
            },
        };
        self.status.set_text(&text);
        set_status_class(&self.status, class);

        // Record / playback controls.
        let running = sh.state == CaptureState::Running;
        let (rec_enabled, rec_text, play_enabled, play_text, note) = match &sh.record {
            RecordState::Idle => (running, format!("● Record {RECORD_SECS} s"), false, "▶ Play back", String::new()),
            RecordState::Recording { frames, target } => {
                let secs = *frames as f32 / (*target as f32 / RECORD_SECS as f32);
                (false, "● Recording…".into(), false, "▶ Play back", format!("{secs:.1} / {RECORD_SECS} s"))
            }
            RecordState::Recorded => (running, format!("● Record {RECORD_SECS} s"), true, "▶ Play back", "Recorded".into()),
            RecordState::Playing => (false, format!("● Record {RECORD_SECS} s"), true, "■ Stop", "Playing…".into()),
            RecordState::Error(e) => (running, format!("● Record {RECORD_SECS} s"), sh.has_recording(), "▶ Play back", e.clone()),
        };
        self.record_btn.set_sensitive(rec_enabled);
        self.record_btn.set_label(&rec_text);
        self.play_btn.set_sensitive(play_enabled);
        self.play_btn.set_label(play_text);
        self.record_label.set_text(&note);
    }
}

fn set_status_class(l: &gtk::Label, class: &str) {
    for c in ["success", "warning", "error", "dim-label"] {
        l.remove_css_class(c);
    }
    l.add_css_class(class);
}

fn source_subtitle(s: &SourceEntry) -> String {
    let kind = match (s.bus.as_deref(), s.form_factor.as_deref()) {
        (Some("bluetooth"), _) => "Bluetooth",
        (Some("usb"), _) => "USB",
        (_, Some("internal")) => "Internal",
        (_, Some("webcam")) => "Webcam",
        (_, Some("headset")) => "Headset",
        (Some("pci"), _) => "Internal",
        _ => "Input",
    };
    let ch = if s.channels == 1 { "mono".to_string() } else { format!("{}ch", s.channels) };
    format!("{kind} · {:.1} kHz · {ch}", s.rate as f32 / 1000.0)
}

fn channel_names(s: &SourceEntry) -> Vec<String> {
    let mut names: Vec<String> = s
        .channel_map
        .split(',')
        .map(|n| match n.trim() {
            "front-left" => "L".to_string(),
            "front-right" => "R".to_string(),
            "mono" => "M".to_string(),
            other => other.to_string(),
        })
        .collect();
    names.resize(s.channels.max(1) as usize, String::new());
    for (i, n) in names.iter_mut().enumerate() {
        if n.is_empty() {
            *n = format!("{}", i + 1);
        }
    }
    names
}

fn db_to_frac(db: f32) -> f64 {
    (((db - METER_MIN_DB) / -METER_MIN_DB).clamp(0.0, 1.0)) as f64
}

fn draw_meters(area: &gtk::DrawingArea, cr: &gtk::cairo::Context, w: i32, _h: i32, data: &MeterData) {
    let fg = area.color();
    let label_w = 28.0;
    let value_w = 70.0;
    let bar_x = label_w;
    let bar_w = (w as f64 - label_w - value_w).max(10.0);
    cr.select_font_face("monospace", gtk::cairo::FontSlant::Normal, gtk::cairo::FontWeight::Normal);
    cr.set_font_size(13.0);

    for (i, lv) in data.levels.iter().enumerate() {
        let y = i as f64 * METER_ROW_H as f64 + 4.0;
        let bh = METER_ROW_H as f64 - 8.0;

        cr.set_source_rgba(fg.red() as f64, fg.green() as f64, fg.blue() as f64, 1.0);
        cr.move_to(2.0, y + bh - 3.0);
        let _ = cr.show_text(data.names.get(i).map(String::as_str).unwrap_or(""));

        // Track.
        cr.set_source_rgba(fg.red() as f64, fg.green() as f64, fg.blue() as f64, 0.12);
        cr.rectangle(bar_x, y, bar_w, bh);
        let _ = cr.fill();

        // Level in three zones: green / amber / red.
        let zones = [(METER_MIN_DB, AMBER_DB, (0.20, 0.70, 0.35)), (AMBER_DB, RED_DB, (0.90, 0.65, 0.10)), (RED_DB, 0.0, (0.85, 0.20, 0.20))];
        for (lo, hi, (r, g, b)) in zones {
            let top = lv.peak_db.min(hi);
            if top <= lo {
                continue;
            }
            let x0 = db_to_frac(lo) * bar_w;
            let x1 = db_to_frac(top) * bar_w;
            cr.set_source_rgb(r, g, b);
            cr.rectangle(bar_x + x0, y, x1 - x0, bh);
            let _ = cr.fill();
        }

        // RMS tick and held peak.
        cr.set_source_rgba(fg.red() as f64, fg.green() as f64, fg.blue() as f64, 0.7);
        let rx = bar_x + db_to_frac(lv.rms_db) * bar_w;
        cr.rectangle(rx - 1.0, y + bh * 0.3, 2.0, bh * 0.4);
        let _ = cr.fill();
        if lv.held_peak_db > METER_MIN_DB {
            let px = bar_x + db_to_frac(lv.held_peak_db) * bar_w;
            cr.set_source_rgba(fg.red() as f64, fg.green() as f64, fg.blue() as f64, 1.0);
            cr.rectangle(px - 1.0, y, 2.0, bh);
            let _ = cr.fill();
        }

        cr.set_source_rgba(fg.red() as f64, fg.green() as f64, fg.blue() as f64, 1.0);
        cr.move_to(bar_x + bar_w + 8.0, y + bh - 3.0);
        let txt = if lv.peak_db <= FLOOR_DB { "  -∞ dB".to_string() } else { format!("{:6.1} dB", lv.peak_db) };
        let _ = cr.show_text(&txt);
    }
}

fn draw_wave(area: &gtk::DrawingArea, cr: &gtk::cairo::Context, w: i32, h: i32, cols: &[(f32, f32)]) {
    let fg = area.color();
    let (w, h) = (w as f64, h as f64);
    let mid = h / 2.0;

    cr.set_source_rgba(fg.red() as f64, fg.green() as f64, fg.blue() as f64, 0.2);
    cr.set_line_width(1.0);
    cr.move_to(0.0, mid);
    cr.line_to(w, mid);
    let _ = cr.stroke();

    if cols.is_empty() {
        return;
    }
    cr.set_source_rgb(0.20, 0.55, 0.85);
    let step = w / crate::analysis::WAVEFORM_COLUMNS as f64;
    // Newest column sits at the right edge.
    let offset = (crate::analysis::WAVEFORM_COLUMNS - cols.len()) as f64 * step;
    for (i, (lo, hi)) in cols.iter().enumerate() {
        let x = offset + i as f64 * step + step / 2.0;
        let y0 = mid - (hi.clamp(-1.0, 1.0) as f64) * mid;
        let y1 = mid - (lo.clamp(-1.0, 1.0) as f64) * mid;
        cr.move_to(x, y0);
        cr.line_to(x, y1.max(y0 + 1.0));
    }
    cr.set_line_width(step.max(1.0));
    let _ = cr.stroke();
}
