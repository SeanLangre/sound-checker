mod activity;
mod autostart;
mod analysis;
mod capture;
mod names;
mod pulse;
mod reconnect;
mod tray;
mod ui;
mod wireplumber;

use adw::prelude::*;

const APP_ID: &str = "dev.soundchecker.SoundChecker";

fn main() -> gtk::glib::ExitCode {
    // `--background` (used by the autostart entry): start in the tray without
    // a window and reconnect remembered Bluetooth audio devices.
    let background = std::env::args().any(|a| a == "--background");
    if background {
        std::thread::spawn(reconnect::run);
    }
    let app = adw::Application::builder().application_id(APP_ID).build();
    app.connect_activate(move |app| ui::build(app, background));
    // Arguments are handled above; keep GApplication from rejecting them.
    app.run_with_args::<&str>(&[])
}
