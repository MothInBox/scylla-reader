//! Binary entry point — panic hook, then delegate to App.

pub mod app;
pub mod db;
pub mod input;
pub mod library;
pub mod messenger;
pub mod models;
pub mod plugin_config;
pub mod scrapers;
pub mod settings;
pub mod state;
pub mod ui;
pub mod worker;

use crossterm::{
    ExecutableCommand,
    terminal::{LeaveAlternateScreen, disable_raw_mode},
};
use std::io::stdout;

fn cleanup() {
    let _ = stdout().execute(LeaveAlternateScreen);
    let _ = disable_raw_mode();
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    std::panic::set_hook(Box::new(|info| {
        cleanup();
        eprintln!("Application panicked: {}", info);
    }));

    let mut app = app::App::new()?;
    app.run()?;

    cleanup();
    Ok(())
}
