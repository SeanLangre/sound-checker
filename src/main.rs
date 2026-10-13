mod activity;
mod analysis;
mod capture;
mod pulse;
mod ui;

use adw::prelude::*;

const APP_ID: &str = "dev.soundchecker.SoundChecker";

fn main() -> gtk::glib::ExitCode {
    let app = adw::Application::builder().application_id(APP_ID).build();
    app.connect_activate(ui::build);
    app.run()
}
