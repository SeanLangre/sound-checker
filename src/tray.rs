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
        "audio-input-microphone".into()
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
