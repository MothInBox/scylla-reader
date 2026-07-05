//! Runtime settings — reader mode, scraping domain, plugin configs, debug logging.

pub mod fields;
pub use fields::SettingsField;

use crate::plugin_config::PluginConfig;
use serde::{Deserialize, Serialize};
use std::fs::OpenOptions;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};

pub static DEBUG_ENABLED: AtomicBool = AtomicBool::new(false);

pub fn log_file() -> std::path::PathBuf {
    let base = dirs::state_dir().unwrap_or_else(|| std::path::PathBuf::from("/tmp"));
    base.join("scylla-reader").join("scylla-reader.log")
}

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
    let path = log_file();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
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

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
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

#[derive(Serialize, Deserialize)]
struct PersistedSettings {
    rate_limit_secs: u64,
    debug_log: bool,
    reader_mode: ReaderMode,
}

impl Default for PersistedSettings {
    fn default() -> Self {
        Self {
            rate_limit_secs: 2,
            debug_log: false,
            reader_mode: ReaderMode::Paged,
        }
    }
}

fn settings_path() -> std::path::PathBuf {
    crate::plugin_config::config_dir().join("settings.json")
}

fn compiled_defaults() -> PersistedSettings {
    PersistedSettings {
        rate_limit_secs: option_env!("SCYLLA_RATE_LIMIT")
            .and_then(|s| s.parse().ok())
            .unwrap_or(2),
        debug_log: option_env!("SCYLLA_DEBUG_LOG")
            .map(|s| s == "true")
            .unwrap_or(false),
        reader_mode: match option_env!("SCYLLA_READER_MODE") {
            Some("Scrollable") => ReaderMode::Scrollable,
            _ => ReaderMode::Paged,
        },
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
    pub fn save(&self) {
        let persisted = PersistedSettings {
            rate_limit_secs: self.rate_limit_secs,
            debug_log: self.debug_log,
            reader_mode: self.reader_mode.clone(),
        };
        let path = settings_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(contents) = serde_json::to_string_pretty(&persisted) {
            let _ = std::fs::write(&path, contents);
        }
    }

    #[cfg(not(test))]
    fn load() -> Option<PersistedSettings> {
        let path = settings_path();
        let contents = std::fs::read_to_string(&path).ok()?;
        serde_json::from_str(&contents).ok()
    }

    pub fn new() -> Self {
        #[allow(unused_mut)]
        let mut merged = compiled_defaults();

        #[cfg(not(test))]
        if let Some(user) = Self::load() {
            merged.rate_limit_secs = user.rate_limit_secs;
            merged.debug_log = user.debug_log;
            merged.reader_mode = user.reader_mode;
        }

        if merged.debug_log {
            set_debug(true);
        }

        Self {
            rate_limit_secs: merged.rate_limit_secs,
            selected_field: 0,
            editing: false,
            edit_buffer: String::new(),
            settings_page: SettingsPage::Main,
            debug_log: merged.debug_log,
            reader_mode: merged.reader_mode,
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
        let path = log_file();
        let content = std::fs::read_to_string(&path).unwrap_or_default();
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
