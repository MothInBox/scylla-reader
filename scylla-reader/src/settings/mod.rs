//! Runtime settings — reader mode, scraping domain, plugin configs, debug logging.

pub mod fields;
pub use fields::SettingsField;

use crate::plugin_config::PluginConfig;
use std::fs::OpenOptions;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};

pub static DEBUG_ENABLED: AtomicBool = AtomicBool::new(false);
pub const LOG_FILE: &str = "/tmp/scylla-reader.log";

pub enum LogLevel {
    Error,
    Debug,
}

impl std::fmt::Display for LogLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            LogLevel::Error => write!(f, "ERROR"),
            LogLevel::Debug => write!(f, "DEBUG"),
        }
    }
}

fn timestamp() -> String {
    let dur = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let total = dur.as_secs();
    let h = (total / 3600) % 24;
    let m = (total / 60) % 60;
    let s = total % 60;
    format!("{:02}:{:02}:{:02} UTC", h, m, s)
}

pub fn log(level: LogLevel, module: &str, msg: &str) {
    let enabled = match level {
        LogLevel::Error => true,
        LogLevel::Debug => DEBUG_ENABLED.load(Ordering::Relaxed),
    };
    if !enabled {
        return;
    }
    let ts = timestamp();
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(LOG_FILE) {
        let _ = writeln!(file, "[{}] [{}] [{}] {}", ts, level, module, msg);
    }
}

#[derive(Debug, PartialEq, Clone)]
pub enum SettingsPage {
    Main,
    DebugLog,
    PluginList,
    PluginFields,
    PluginFieldEdit,
}

#[derive(Debug, PartialEq, Clone)]
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
    pub debug_log: bool,
    pub reader_mode: ReaderMode,
    pub plugin_configs: Vec<PluginConfig>,
    pub selected_plugin: usize,
    pub selected_plugin_field: usize,
    pub plugin_field_editing: bool,
    pub plugin_field_buffer: String,
    pub log_scroll: usize,
    pub log_lines: Vec<String>,
}

impl Settings {
    pub fn new() -> Self {
        Self {
            rate_limit_secs: 2,
            selected_field: 0,
            editing: false,
            edit_buffer: String::new(),
            settings_page: SettingsPage::Main,
            debug_log: false,
            reader_mode: ReaderMode::Paged,
            plugin_configs: PluginConfig::discover_all(),
            selected_plugin: 0,
            selected_plugin_field: 0,
            plugin_field_editing: false,
            plugin_field_buffer: String::new(),
            log_scroll: 0,
            log_lines: Vec::new(),
        }
    }

    pub fn reload_plugins(&mut self) {
        self.plugin_configs = PluginConfig::discover_all();
    }

    pub fn save_current_field(&mut self) -> Result<(), String> {
        let Some(config) = self.plugin_configs.get_mut(self.selected_plugin) else {
            return Ok(());
        };
        let is_cookie = self.selected_plugin_field == config.schema.len();
        if is_cookie {
            config.cookies = self.plugin_field_buffer.clone();
            return config.save();
        }
        let key = config
            .schema
            .get(self.selected_plugin_field)
            .map(|f| f.key.clone());
        let Some(key) = key else {
            return Ok(());
        };
        if let Some(field) = config.schema.get(self.selected_plugin_field) {
            if field.field_type == "number"
                && !self.plugin_field_buffer.is_empty()
                && self.plugin_field_buffer.parse::<f64>().is_err()
            {
                return Err("Invalid number".to_string());
            }
        }
        config.update_value(&key, self.plugin_field_buffer.clone());
        config.save()
    }

    pub fn reload_log(&mut self) {
        let content = std::fs::read_to_string(LOG_FILE).unwrap_or_default();
        self.log_lines = content.lines().map(|l| l.to_string()).collect();
    }

    pub fn field_value(&self, field: &SettingsField) -> String {
        match field {
            SettingsField::RateLimit => self.rate_limit_secs.to_string(),
            SettingsField::DebugLog => {
                if self.debug_log {
                    "ON".to_string()
                } else {
                    "OFF".to_string()
                }
            }
            SettingsField::ReaderMode => self.reader_mode.to_string(),
            SettingsField::Plugins => {
                format!("{} domain(s)", self.plugin_configs.len())
            }
        }
    }
}

pub fn set_debug(enabled: bool) {
    DEBUG_ENABLED.store(enabled, Ordering::Relaxed);
}



#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reader_mode_toggle() {
        assert_eq!(ReaderMode::Paged.toggle(), ReaderMode::Scrollable);
        assert_eq!(ReaderMode::Scrollable.toggle(), ReaderMode::Paged);
    }

    #[test]
    fn test_reader_mode_display() {
        assert_eq!(format!("{}", ReaderMode::Paged), "Paged");
        assert_eq!(format!("{}", ReaderMode::Scrollable), "Scrollable");
    }

    #[test]
    fn test_settings_defaults() {
        let s = Settings::new();
        assert_eq!(s.rate_limit_secs, 2);
        assert_eq!(s.reader_mode, ReaderMode::Paged);
        assert!(!s.debug_log);
        assert_eq!(s.selected_field, 0);
        assert_eq!(s.settings_page, SettingsPage::Main);
        assert_eq!(s.selected_plugin, 0);
        assert_eq!(s.selected_plugin_field, 0);
        assert!(!s.plugin_field_editing);
    }

    #[test]
    fn test_field_value_rate_limit() {
        let s = Settings::new();
        assert_eq!(s.field_value(&SettingsField::RateLimit), "2");
    }

    #[test]
    fn test_field_value_debug_log() {
        let mut s = Settings::new();
        assert_eq!(s.field_value(&SettingsField::DebugLog), "OFF");
        s.debug_log = true;
        assert_eq!(s.field_value(&SettingsField::DebugLog), "ON");
    }

    #[test]
    fn test_field_value_reader_mode() {
        let mut s = Settings::new();
        assert_eq!(s.field_value(&SettingsField::ReaderMode), "Paged");
        s.reader_mode = ReaderMode::Scrollable;
        assert_eq!(s.field_value(&SettingsField::ReaderMode), "Scrollable");
    }

    #[test]
    fn test_set_debug_toggle() {
        assert!(!DEBUG_ENABLED.load(Ordering::Relaxed));
        set_debug(true);
        assert!(DEBUG_ENABLED.load(Ordering::Relaxed));
        set_debug(false);
        assert!(!DEBUG_ENABLED.load(Ordering::Relaxed));
    }
}
