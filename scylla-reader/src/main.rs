//! Binary entry point — panic hook, then delegate to App.

pub mod app;
pub mod db;
pub mod event;
pub mod input;
pub mod key_handler;
pub mod library;
pub mod messenger;
pub mod models;
pub mod plugin_config;
pub mod scrapers;
pub mod settings;
pub mod state;
#[cfg(test)]
pub mod test_helpers;
pub mod textwrap;
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
        let thread_name = std::thread::current()
            .name()
            .unwrap_or("<unnamed>")
            .to_string();

        crate::settings::log(
            crate::settings::LogLevel::Error,
            "PANIC",
            &format!("thread '{}' panicked: {}", thread_name, info),
        );

        if thread_name == "main" {
            cleanup();
            eprintln!("Application panicked: {}", info);
        }
    }));

    crate::settings::log(
        crate::settings::LogLevel::Debug,
        "MAIN",
        "Application starting",
    );
    let mut app = app::App::new()?;
    let result = app.run();
    crate::settings::log(
        crate::settings::LogLevel::Debug,
        "MAIN",
        "Application shutting down",
    );
    cleanup();
    result?;
    Ok(())
}
