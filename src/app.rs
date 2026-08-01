//! Application entry point — GTK4 Application setup.

use gtk4::Application;
use gtk4::prelude::*;

use crate::ui;

const APP_ID: &str = "tech.goodcol.vmlaunch";

pub fn run() {
    let app = Application::builder().application_id(APP_ID).build();

    app.connect_activate(|app| {
        // Clicking the dock icon again re-activates the app. Without this the
        // second activation builds a whole second window and SPICE session
        // against the same VM.
        if let Some(window) = app.active_window() {
            window.present();
            return;
        }
        ui::build_window(app);
    });

    app.run();
}
