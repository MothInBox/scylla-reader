pub mod fields;

pub use fields::SettingsField;

use crate::cookie_store::CookieStore;
use std::fs::OpenOptions;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};

pub static DEBUG_ENABLED: AtomicBool = AtomicBool::new(false);
pub const LOG_FILE: &str = "/tmp/scylla-reader.log";

#[derive(PartialEq, Clone)]
pub enum SettingsPage {
    Main,
    CookieList,
    CookieEdit,
}

#[derive(PartialEq, Clone)]
pub enum ReaderMode {
    Paged,
    Scrollable,
}

impl ReaderMode {
    pub fn toggle(&self) -> ReaderMode {
        match self {
            ReaderMode::Paged => ReaderMode::Scrollable,
            ReaderMode::Scrollable => ReaderMode::Paged,
        }
    }
}

impl std::fmt::Display for ReaderMode {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            ReaderMode::Paged => write!(f, "Paged"),
            ReaderMode::Scrollable => write!(f, "Scrollable"),
        }
    }
}

pub struct Settings {
    pub rate_limit_secs: u64,
    pub selected_field: usize,
    pub editing: bool,
    pub edit_buffer: String,
    pub settings_page: SettingsPage,
    pub cookie_stores: Vec<CookieStore>,
    pub selected_cookie: usize,
    pub cookie_edit_buffer: String,
    pub debug_log: bool,
    pub reader_mode: ReaderMode,
}

impl Settings {
    pub fn new() -> Self {
        Self {
            rate_limit_secs: 2,
            selected_field: 0,
            editing: false,
            edit_buffer: String::new(),
            settings_page: SettingsPage::Main,
            cookie_stores: CookieStore::discover_all(),
            selected_cookie: 0,
            cookie_edit_buffer: String::new(),
            debug_log: false,
            reader_mode: ReaderMode::Paged,
        }
    }

    pub fn reload_cookies(&mut self) {
        self.cookie_stores = CookieStore::discover_all();
    }

    pub fn save_current_cookie(&mut self) {
        if let Some(store) = self.cookie_stores.get(self.selected_cookie) {
            if let Err(e) = store.save(&self.cookie_edit_buffer) {
                log_debug(&format!("Failed to save cookie: {}", e));
            }
        }
    }

    pub fn field_value(&self, field: &SettingsField) -> String {
        match field {
            SettingsField::Cookies => format!("{} domain(s) configured", self.cookie_stores.len()),
            SettingsField::RateLimit => self.rate_limit_secs.to_string(),
            SettingsField::DebugLog => {
                if self.debug_log {
                    "ON".to_string()
                } else {
                    "OFF".to_string()
                }
            }
            SettingsField::ReaderMode => self.reader_mode.to_string(),
        }
    }
}

pub fn set_debug(enabled: bool) {
    DEBUG_ENABLED.store(enabled, Ordering::Relaxed);
}

pub fn log_debug(msg: &str) {
    if !DEBUG_ENABLED.load(Ordering::Relaxed) {
        return;
    }
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(LOG_FILE) {
        let _ = writeln!(file, "[DEBUG] {}", msg);
    }
}
