//! Application entry point — GTK4 Application setup.

use gtk4::Application;
use gtk4::prelude::*;

use crate::ui;

const APP_ID: &str = "tech.goodcol.vmlaunch";

pub fn run() {
    let app = Application::builder().application_id(APP_ID).build();

    app.connect_activate(|app| {
        ui::build_window(app);
    });

    app.run();
}
