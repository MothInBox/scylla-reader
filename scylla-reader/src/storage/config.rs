use scylla_core::types::LibraryConfig;

const CONFIG_FILE: &str = "libraries.json";

/// Reader-side config-dir override. Separate from `scylla_core::paths` and
/// `crate::settings` overrides so tests can redirect `libraries.json` writes
/// without conflicting with plugin-config tests.
static CONFIG_DIR_OVERRIDE: std::sync::OnceLock<Option<std::path::PathBuf>> =
    std::sync::OnceLock::new();

/// Redirect `libraries_path()` to `{dir}/scylla-reader/libraries.json`.
/// Set-once: the first caller wins, later calls are ignored.
pub fn set_config_dir_override(dir: std::path::PathBuf) {
    let _ = CONFIG_DIR_OVERRIDE.set(Some(dir));
}

pub fn libraries_path() -> std::path::PathBuf {
    let config_dir = CONFIG_DIR_OVERRIDE
        .get()
        .and_then(|o| o.as_ref())
        .cloned()
        .unwrap_or_else(|| {
            dirs::config_local_dir().unwrap_or_else(|| std::path::PathBuf::from("."))
        })
        .join("scylla-reader");
    config_dir.join(CONFIG_FILE)
}

pub fn load_libraries() -> Vec<LibraryConfig> {
    let path = libraries_path();
    if !path.exists() {
        return vec![LibraryConfig {
            name: "local".into(),
            url: "http://127.0.0.1:8080".into(),
            default: true,
        }];
    }
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_libraries(libraries: &[LibraryConfig]) -> Result<(), String> {
    let path = libraries_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(libraries).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_libraries_path_uses_config_dir_override() {
        let dir = std::env::temp_dir().join(format!("scylla-test-config-{}", std::process::id()));
        set_config_dir_override(dir.clone());
        assert_eq!(
            libraries_path(),
            dir.join("scylla-reader").join(CONFIG_FILE)
        );
    }
}
