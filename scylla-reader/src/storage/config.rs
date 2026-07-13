use scylla_core::types::LibraryConfig;

const CONFIG_FILE: &str = "libraries.json";

pub fn libraries_path() -> std::path::PathBuf {
    let config_dir = dirs::config_local_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
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
