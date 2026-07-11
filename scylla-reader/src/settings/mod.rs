//! Runtime settings — reader mode, scraping domain, plugin configs, debug logging.

pub mod fields;
pub use fields::SettingsField;

use crate::plugin_config::PluginConfig;
use serde::{Deserialize, Serialize};
use std::fs::OpenOptions;
use std::io::Write;
use std::sync::OnceLock;
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
    max_workers: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    config_dir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    data_dir: Option<String>,
}

impl Default for PersistedSettings {
    fn default() -> Self {
        Self {
            rate_limit_secs: 2,
            debug_log: false,
            reader_mode: ReaderMode::Paged,
            max_workers: 4,
            config_dir: None,
            data_dir: None,
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
        max_workers: option_env!("SCYLLA_MAX_WORKERS")
            .and_then(|s| s.parse().ok())
            .unwrap_or(4),
        config_dir: None,
        data_dir: None,
    }
}

pub static CONFIG_DIR_OVERRIDE: OnceLock<Option<std::path::PathBuf>> = OnceLock::new();
pub static DATA_DIR_OVERRIDE: OnceLock<Option<std::path::PathBuf>> = OnceLock::new();

pub fn set_path_overrides(
    config_dir: Option<std::path::PathBuf>,
    data_dir: Option<std::path::PathBuf>,
) {
    let _ = CONFIG_DIR_OVERRIDE.set(config_dir);
    let _ = DATA_DIR_OVERRIDE.set(data_dir);
}

pub struct Settings {
    pub rate_limit_secs: u64,
    pub debug_log: bool,
    pub reader_mode: ReaderMode,
    pub max_workers: u8,
    pub plugin_configs: Vec<PluginConfig>,
}

impl Default for Settings {
    fn default() -> Self {
        Self::new()
    }
}

impl Settings {
    pub fn save(&self) {
        let config_dir = CONFIG_DIR_OVERRIDE
            .get()
            .and_then(|o| o.as_ref())
            .map(|p| p.to_string_lossy().to_string());
        let data_dir = DATA_DIR_OVERRIDE
            .get()
            .and_then(|o| o.as_ref())
            .map(|p| p.to_string_lossy().to_string());
        let persisted = PersistedSettings {
            rate_limit_secs: self.rate_limit_secs,
            debug_log: self.debug_log,
            reader_mode: self.reader_mode.clone(),
            max_workers: self.max_workers,
            config_dir,
            data_dir,
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
            merged.max_workers = user.max_workers;
            set_path_overrides(
                user.config_dir.map(std::path::PathBuf::from),
                user.data_dir.map(std::path::PathBuf::from),
            );
        }

        if merged.debug_log {
            set_debug(true);
        }

        Self {
            rate_limit_secs: merged.rate_limit_secs,
            debug_log: merged.debug_log,
            reader_mode: merged.reader_mode,
            max_workers: merged.max_workers,
            plugin_configs: PluginConfig::discover_all(),
        }
    }

    pub fn reload_plugins(&mut self) {
        self.plugin_configs = PluginConfig::discover_all();
    }

    pub fn save_current_field(
        &mut self,
        selected_plugin: usize,
        selected_plugin_field: usize,
        plugin_field_buffer: &str,
    ) -> Result<(), String> {
        let Some(config) = self.plugin_configs.get_mut(selected_plugin) else {
            return Ok(());
        };
        let is_cookie = selected_plugin_field == config.schema.len();
        if is_cookie {
            config.cookies = plugin_field_buffer.to_string();
            return config.save();
        }
        let key = config
            .schema
            .get(selected_plugin_field)
            .map(|f| f.key.clone());
        let Some(key) = key else {
            return Ok(());
        };
        if let Some(field) = config.schema.get(selected_plugin_field)
            && field.field_type == "number"
            && !plugin_field_buffer.is_empty()
            && plugin_field_buffer.parse::<f64>().is_err()
        {
            return Err("Invalid number".to_string());
        }
        config.update_value(&key, plugin_field_buffer.to_string());
        config.save()
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

    #[test]
    fn test_persisted_settings_default() {
        let p = PersistedSettings::default();
        assert_eq!(p.rate_limit_secs, 2);
        assert!(!p.debug_log);
        assert_eq!(p.reader_mode, ReaderMode::Paged);
        assert_eq!(p.max_workers, 4);
    }

    #[test]
    fn test_persisted_settings_roundtrip() {
        let p = PersistedSettings {
            rate_limit_secs: 42,
            debug_log: true,
            reader_mode: ReaderMode::Scrollable,
            max_workers: 8,
            config_dir: None,
            data_dir: None,
        };
        let json = serde_json::to_string(&p).unwrap();
        let back: PersistedSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(back.rate_limit_secs, 42);
        assert!(back.debug_log);
        assert_eq!(back.reader_mode, ReaderMode::Scrollable);
        assert_eq!(back.max_workers, 8);
    }

    #[test]
    fn test_compiled_defaults_fallback() {
        let d = compiled_defaults();
        assert_eq!(d.rate_limit_secs, 2);
        assert!(!d.debug_log);
        assert_eq!(d.reader_mode, ReaderMode::Paged);
        assert_eq!(d.max_workers, 4);
    }

    #[test]
    fn test_path_overrides() {
        assert!(CONFIG_DIR_OVERRIDE.get().is_none());
        assert!(DATA_DIR_OVERRIDE.get().is_none());

        set_path_overrides(
            Some(std::path::PathBuf::from("/custom/config")),
            Some(std::path::PathBuf::from("/custom/data")),
        );

        assert_eq!(
            CONFIG_DIR_OVERRIDE.get(),
            Some(&Some(std::path::PathBuf::from("/custom/config")))
        );
        assert_eq!(
            DATA_DIR_OVERRIDE.get(),
            Some(&Some(std::path::PathBuf::from("/custom/data")))
        );
    }

    #[test]
    fn test_save_roundtrip() {
        let mut settings = Settings::new();
        settings.rate_limit_secs = 99;
        settings.debug_log = true;
        settings.reader_mode = ReaderMode::Scrollable;

        let persisted = PersistedSettings {
            rate_limit_secs: settings.rate_limit_secs,
            debug_log: settings.debug_log,
            reader_mode: settings.reader_mode.clone(),
            max_workers: settings.max_workers,
            config_dir: None,
            data_dir: None,
        };
        let json = serde_json::to_string_pretty(&persisted).unwrap();
        let back: PersistedSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(back.rate_limit_secs, 99);
        assert!(back.debug_log);
        assert_eq!(back.reader_mode, ReaderMode::Scrollable);
        assert_eq!(back.max_workers, 4);
    }
}
