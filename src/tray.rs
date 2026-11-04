//! System tray icon (StatusNotifierItem), so the app can live in the tray.
//!
//! The tray runs on its own thread; clicks reach the GTK thread through a
//! channel that `ui` polls.

use std::sync::mpsc::{self, Receiver, Sender};

use ksni::blocking::TrayMethods;

pub enum Command {
    Show,
    ScanBluetooth,
    Quit,
}

struct Tray(Sender<Command>);

impl ksni::Tray for Tray {
    fn id(&self) -> String {
        "dev.soundchecker.SoundChecker".into()
    }
    fn title(&self) -> String {
        "Linux Sound Checker".into()
    }
    fn icon_name(&self) -> String {
        // Empty so hosts use `icon_pixmap` instead of a theme icon.
        String::new()
    }
    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        [22, 32, 48, 64].into_iter().map(render_icon).collect()
    }
    fn activate(&mut self, _x: i32, _y: i32) {
        let _ = self.0.send(Command::Show);
    }
    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::StandardItem;
        vec![
            StandardItem {
                label: "Show Sound Checker".into(),
                activate: Box::new(|t: &mut Self| {
                    let _ = t.0.send(Command::Show);
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Scan for Bluetooth devices".into(),
                activate: Box::new(|t: &mut Self| {
                    let _ = t.0.send(Command::ScanBluetooth);
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Quit".into(),
                activate: Box::new(|t: &mut Self| {
                    let _ = t.0.send(Command::Quit);
                }),
                ..Default::default()
            }
            .into(),
        ]
    }
}

/// Start the tray icon. `None` when no tray host is running (then the window
/// must not hide itself, or it would be unreachable). The handle keeps it alive.
pub fn start() -> Option<(ksni::blocking::Handle<impl ksni::Tray>, Receiver<Command>)> {
    let (tx, rx) = mpsc::channel();
    let handle = Tray(tx).spawn().ok()?;
    Some((handle, rx))
}

/// Signed distance to a rounded rect centred at (cx, cy) with half-size (hx, hy).
fn rrect_dist(x: f32, y: f32, cx: f32, cy: f32, hx: f32, hy: f32, r: f32) -> f32 {
    let qx = (x - cx).abs() - (hx - r);
    let qy = (y - cy).abs() - (hy - r);
    qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - r
}

/// Draw the tray glyph: the app's blue badge with a white level-meter
/// waveform (distinct from the stock microphone icon), antialiased.
fn render_icon(size: i32) -> ksni::Icon {
    // Bar heights as a fraction of the badge, in 128-unit space.
    const BARS: [f32; 5] = [30.0, 58.0, 88.0, 50.0, 24.0];
    let k = size as f32 / 128.0;
    let mut data = Vec::with_capacity((size * size * 4) as usize);
    for py in 0..size {
        for px in 0..size {
            let (x, y) = ((px as f32 + 0.5) / k, (py as f32 + 0.5) / k);
            let cover = |d: f32| (0.5 - d * k).clamp(0.0, 1.0);
            let bg = cover(rrect_dist(x, y, 64.0, 64.0, 64.0, 64.0, 28.0));
            let mut fg = 0.0f32;
            for (i, h) in BARS.iter().enumerate() {
                let cx = 22.0 + i as f32 * 21.0;
                fg = fg.max(cover(rrect_dist(x, y, cx, 64.0, 7.0, h / 2.0, 7.0)));
            }
            // Blend white bars over the blue (#3584e4) badge; ARGB32, network order.
            let mix = |b: f32| (b + (255.0 - b) * fg).round() as u8;
            data.extend_from_slice(&[(bg * 255.0).round() as u8, mix(0x35 as f32), mix(0x84 as f32), mix(0xe4 as f32)]);
        }
    }
    ksni::Icon { width: size, height: size, data }
}
