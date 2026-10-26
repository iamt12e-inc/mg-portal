mod config;
mod credentials;
mod network;
mod portal;
mod ui;
mod worker;

use gtk::gio;
use gtk::prelude::*;
use std::sync::{Arc, Mutex};

const APP_ID: &str = "io.github.tanaka_mambinge.MugonatPortal";

fn main() -> gtk::glib::ExitCode {
    let settings = match config::Settings::load() {
        Ok(settings) => settings,
        Err(error) => {
            eprintln!("Could not load app settings; using defaults: {error}");
            config::Settings::default()
        }
    };

    let app = gtk::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::HANDLES_COMMAND_LINE)
        .build();
    ui::install(&app, Arc::new(Mutex::new(settings)));
    app.run()
}
