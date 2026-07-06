use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use scylla_plugin_api::ConfigField;

pub struct PluginConfig {
    pub domain: String,
    pub schema: Vec<ConfigField>,
    pub accepts_cookies: bool,
    pub values: HashMap<String, String>,
    pub cookies: String,
    path: PathBuf,
}

impl PluginConfig {
    pub fn for_domain(domain: &str, schema: Vec<ConfigField>, accepts_cookies: bool) -> Self {
        let path = config_dir().join(format!("{}.json", domain));
        let (values, cookies) = if path.exists() {
            Self::load_from_disk(&path, &schema)
        } else {
            (Self::default_values(&schema), String::new())
        };
        Self {
            domain: domain.to_string(),
            schema,
            accepts_cookies,
            values,
            cookies,
            path,
        }
    }

    pub fn discover_all() -> Vec<PluginConfig> {
        Self::migrate_txt_files();
        let dir = config_dir();
        let mut configs: HashMap<String, PluginConfig> = HashMap::new();

        if let Ok(entries) = fs::read_dir(&dir) {
            for entry in entries.flatten() {
                if entry
                    .path()
                    .extension()
                    .map(|x| x == "json")
                    .unwrap_or(false)
                {
                    let path = entry.path();
                    let contents = match fs::read_to_string(&path) {
                        Ok(c) => c,
                        _ => continue,
                    };
                    let json: serde_json::Value = match serde_json::from_str(&contents) {
                        Ok(v) => v,
                        _ => continue,
                    };
                    let schema: Vec<ConfigField> = json
                        .get("_schema")
                        .and_then(|s| serde_json::from_value(s.clone()).ok())
                        .unwrap_or_default();
                    let domain = match path.file_stem().and_then(|s| s.to_str()) {
                        Some(d) => d.to_string(),
                        _ => continue,
                    };
                    let accepts_cookies = json
                        .get("_accepts_cookies")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(true);
                    let (values, cookies) = Self::load_from_disk(&path, &schema);
                    configs.insert(
                        domain.clone(),
                        PluginConfig {
                            domain,
                            schema,
                            accepts_cookies,
                            values,
                            cookies,
                            path,
                        },
                    );
                }
            }
        }

        let plugin_dir = plugins_dir();
        if let Ok(entries) = fs::read_dir(&plugin_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("wasm") {
                    continue;
                }
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str())
                    && let Some(domain) = stem.strip_prefix("plugin-")
                        && !configs.contains_key(domain) {
                            configs.insert(
                                domain.to_string(),
                                PluginConfig {
                                    domain: domain.to_string(),
                                    schema: vec![],
                                    accepts_cookies: true,
                                    values: HashMap::new(),
                                    cookies: String::new(),
                                    path: config_dir().join(format!("{}.json", domain)),
                                },
                            );
                        }
            }
        }

        configs.into_values().collect()
    }

    pub fn is_pending(&self) -> bool {
        !self.path.as_os_str().is_empty() && !self.path.exists()
    }

    pub fn save(&self) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut map = serde_json::Map::new();
        if let Ok(schema_json) = serde_json::to_value(&self.schema) {
            map.insert("_schema".into(), schema_json);
        }
        map.insert(
            "_accepts_cookies".into(),
            serde_json::Value::Bool(self.accepts_cookies),
        );
        map.insert(
            "_cookies".into(),
            serde_json::Value::String(self.cookies.clone()),
        );
        for (k, v) in &self.values {
            map.insert(k.clone(), serde_json::Value::String(v.clone()));
        }
        let json = serde_json::Value::Object(map);
        let contents = serde_json::to_string_pretty(&json).map_err(|e| e.to_string())?;
        fs::write(&self.path, contents).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn update_value(&mut self, key: &str, value: String) {
        self.values.insert(key.to_string(), value);
    }

    pub fn preview(&self) -> String {
        if self.is_pending() {
            return "loading...".to_string();
        }
        let count = self.schema.len();
        let has_cookies = !self.cookies.is_empty()
            && self.cookies.lines().any(|l| {
                let t = l.trim();
                !t.is_empty() && !t.starts_with('#')
            });
        match (count, has_cookies) {
            (0, false) => "No configurable fields".to_string(),
            (0, true) => "Cookies only".to_string(),
            (_, false) => format!("{} field(s)", count),
            (_, true) => format!("{} field(s) + cookies", count),
        }
    }

    pub fn cookies_preview(&self) -> String {
        let trimmed = self.cookies.trim();
        if trimmed.is_empty() {
            "<empty>".to_string()
        } else if trimmed.chars().count() > 40 {
            format!("{}...", trimmed.chars().take(40).collect::<String>())
        } else {
            trimmed.to_string()
        }
    }

    pub fn parse_cookies(&self) -> String {
        self.cookies
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .collect::<Vec<_>>()
            .join("; ")
    }

    pub fn init_config_file(domain: &str, schema: &[ConfigField], accepts_cookies: bool) {
        let path = config_dir().join(format!("{}.json", domain));
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).ok();
        }

        if path.exists() {
            let contents = fs::read_to_string(&path).ok();
            let mut json: serde_json::Value = contents
                .and_then(|c| serde_json::from_str(&c).ok())
                .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));
            let mut changed = false;

            if let Ok(s) = serde_json::to_value(schema)
                && json.get("_schema") != Some(&s)
                    && let Some(obj) = json.as_object_mut() {
                        obj.insert("_schema".into(), s);
                        changed = true;
                    }

            let current_ac = json.get("_accepts_cookies").and_then(|v| v.as_bool());
            if current_ac != Some(accepts_cookies)
                && let Some(obj) = json.as_object_mut() {
                    obj.insert(
                        "_accepts_cookies".into(),
                        serde_json::Value::Bool(accepts_cookies),
                    );
                    changed = true;
                }

            if json.get("_cookies").is_none() {
                let txt_path = config_dir().join(format!("{}.txt", domain));
                if txt_path.exists() {
                    let txt_content = fs::read_to_string(&txt_path).unwrap_or_default();
                    fs::remove_file(&txt_path).ok();
                    if !txt_content.trim().is_empty()
                        && let Some(obj) = json.as_object_mut() {
                            obj.insert("_cookies".into(), serde_json::Value::String(txt_content));
                            changed = true;
                        }
                }
            }

            if let Some(obj) = json.as_object_mut() {
                for field in schema {
                    if !obj.contains_key(&field.key) {
                        obj.insert(
                            field.key.clone(),
                            serde_json::Value::String(field.default.clone()),
                        );
                        changed = true;
                    }
                }
            }

            if changed
                && let Ok(contents) = serde_json::to_string_pretty(&json) {
                    fs::write(&path, contents).ok();
                }
        } else {
            let mut map = serde_json::Map::new();

            if let Ok(s) = serde_json::to_value(schema) {
                map.insert("_schema".into(), s);
            }

            map.insert(
                "_accepts_cookies".into(),
                serde_json::Value::Bool(accepts_cookies),
            );

            for field in schema {
                map.insert(
                    field.key.clone(),
                    serde_json::Value::String(field.default.clone()),
                );
            }

            let txt_path = config_dir().join(format!("{}.txt", domain));
            if txt_path.exists() {
                let txt_content = fs::read_to_string(&txt_path).unwrap_or_default();
                fs::remove_file(&txt_path).ok();
                if !txt_content.trim().is_empty() {
                    map.insert("_cookies".into(), serde_json::Value::String(txt_content));
                }
            }

            if let Ok(contents) = serde_json::to_string_pretty(&serde_json::Value::Object(map)) {
                fs::write(&path, contents).ok();
            }
        }
    }

    fn migrate_txt_files() {
        let dir = config_dir();
        let Ok(entries) = fs::read_dir(&dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("txt") {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if stem == "library" {
                continue;
            }
            let json_path = dir.join(format!("{}.json", stem));
            if json_path.exists() {
                let contents = fs::read_to_string(&json_path).ok();
                let json: Option<serde_json::Value> =
                    contents.and_then(|c| serde_json::from_str(&c).ok());
                let has_cookies = json
                    .as_ref()
                    .and_then(|v| v.as_object())
                    .and_then(|o| o.get("_cookies"))
                    .and_then(|v| v.as_str())
                    .map(|s| !s.is_empty())
                    .unwrap_or(false);
                if !has_cookies {
                    let txt_content = fs::read_to_string(&path).unwrap_or_default();
                    if let Some(mut obj) = json.and_then(|v| v.as_object().cloned()) {
                        obj.insert("_cookies".into(), serde_json::Value::String(txt_content));
                        if let Ok(updated) =
                            serde_json::to_string_pretty(&serde_json::Value::Object(obj))
                        {
                            fs::write(&json_path, updated).ok();
                        }
                    }
                }
            }
            fs::remove_file(&path).ok();
        }
    }

    fn default_values(schema: &[ConfigField]) -> HashMap<String, String> {
        schema
            .iter()
            .map(|f| (f.key.clone(), f.default.clone()))
            .collect()
    }

    fn load_from_disk(path: &PathBuf, schema: &[ConfigField]) -> (HashMap<String, String>, String) {
        let contents = fs::read_to_string(path).ok();
        let json: Option<serde_json::Value> = contents.and_then(|c| serde_json::from_str(&c).ok());
        let mut values = Self::default_values(schema);
        let mut cookies = String::new();
        if let Some(obj) = json.and_then(|v| v.as_object().cloned()) {
            for (k, v) in &obj {
                match k.as_str() {
                    "_schema" | "_cookies" => {}
                    _ => {
                        if let Some(s) = v.as_str() {
                            values.insert(k.clone(), s.to_string());
                        }
                    }
                }
            }
            cookies = obj
                .get("_cookies")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
        }
        (values, cookies)
    }
}

pub fn config_dir() -> PathBuf {
    dirs::config_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("scylla-reader")
}

pub fn plugins_dir() -> PathBuf {
    config_dir().join("plugins")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_preview_no_schema_no_cookies() {
        let config = PluginConfig {
            domain: "test".into(),
            schema: vec![],
            values: HashMap::new(),
            cookies: String::new(),
            accepts_cookies: true,
            path: PathBuf::new(),
        };
        assert_eq!(config.preview(), "No configurable fields");
    }

    #[test]
    fn test_preview_no_schema_with_cookies() {
        let config = PluginConfig {
            domain: "test".into(),
            schema: vec![],
            values: HashMap::new(),
            cookies: "key=value".into(),
            accepts_cookies: true,
            path: PathBuf::new(),
        };
        assert_eq!(config.preview(), "Cookies only");
    }

    #[test]
    fn test_preview_with_schema_no_cookies() {
        let config = PluginConfig {
            domain: "test".into(),
            schema: vec![ConfigField {
                key: "k".into(),
                label: "K".into(),
                field_type: "string".into(),
                default: "".into(),
            }],
            values: HashMap::new(),
            cookies: String::new(),
            accepts_cookies: true,
            path: PathBuf::new(),
        };
        assert_eq!(config.preview(), "1 field(s)");
    }

    #[test]
    fn test_preview_with_schema_and_cookies() {
        let config = PluginConfig {
            domain: "test".into(),
            schema: vec![ConfigField {
                key: "k".into(),
                label: "K".into(),
                field_type: "string".into(),
                default: "".into(),
            }],
            values: HashMap::new(),
            cookies: "key=value".into(),
            accepts_cookies: true,
            path: PathBuf::new(),
        };
        assert_eq!(config.preview(), "1 field(s) + cookies");
    }

    #[test]
    fn test_cookies_preview_empty() {
        let config = PluginConfig {
            domain: "test".into(),
            schema: vec![],
            values: HashMap::new(),
            cookies: String::new(),
            accepts_cookies: true,
            path: PathBuf::new(),
        };
        assert_eq!(config.cookies_preview(), "<empty>");
    }

    #[test]
    fn test_cookies_preview_short() {
        let config = PluginConfig {
            domain: "test".into(),
            schema: vec![],
            values: HashMap::new(),
            cookies: "key=value".into(),
            accepts_cookies: true,
            path: PathBuf::new(),
        };
        assert_eq!(config.cookies_preview(), "key=value");
    }

    #[test]
    fn test_cookies_preview_long_truncated() {
        let long = "abcdefghijklmnopqrstuvwxyz0123456789abcdefghijklmnopqrstuvwxyz";
        let config = PluginConfig {
            domain: "test".into(),
            schema: vec![],
            values: HashMap::new(),
            cookies: long.into(),
            accepts_cookies: true,
            path: PathBuf::new(),
        };
        let preview = config.cookies_preview();
        assert!(preview.ends_with("..."), "got: {}", preview);
        assert!(preview.chars().count() <= 43);
    }

    #[test]
    fn test_parse_cookies_skips_comments_and_blanks() {
        let config = PluginConfig {
            domain: "test".into(),
            schema: vec![],
            values: HashMap::new(),
            cookies: "# comment\nkey1=val1\n\nkey2=val2\n".into(),
            accepts_cookies: true,
            path: PathBuf::new(),
        };
        assert_eq!(config.parse_cookies(), "key1=val1; key2=val2");
    }

    #[test]
    fn test_for_domain_new_creates_default_values() {
        let schema = vec![ConfigField {
            key: "a".into(),
            label: "A".into(),
            field_type: "string".into(),
            default: "default_a".into(),
        }];
        let config = PluginConfig::for_domain("nonexistent_test_domain", schema, true);
        assert_eq!(
            config.values.get("a").map(|v| v.as_str()),
            Some("default_a")
        );
        assert!(config.cookies.is_empty());
    }

    #[test]
    fn test_default_values_matches_schema() {
        let schema = vec![
            ConfigField {
                key: "x".into(),
                label: "X".into(),
                field_type: "number".into(),
                default: "42".into(),
            },
            ConfigField {
                key: "y".into(),
                label: "Y".into(),
                field_type: "string".into(),
                default: "hello".into(),
            },
        ];
        let values = PluginConfig::default_values(&schema);
        assert_eq!(values.get("x").map(|v| v.as_str()), Some("42"));
        assert_eq!(values.get("y").map(|v| v.as_str()), Some("hello"));
        assert_eq!(values.len(), 2);
    }

    #[test]
    fn test_is_pending_empty_path_returns_false() {
        let config = PluginConfig {
            domain: "test".into(),
            schema: vec![],
            accepts_cookies: true,
            values: HashMap::new(),
            cookies: String::new(),
            path: PathBuf::new(),
        };
        assert!(!config.is_pending());
    }

    #[test]
    fn test_preview_pending_shows_loading() {
        let config = PluginConfig {
            domain: "test".into(),
            schema: vec![],
            accepts_cookies: true,
            values: HashMap::new(),
            cookies: String::new(),
            path: PathBuf::from("/nonexistent/path/test.json"),
        };
        assert!(config.is_pending());
        assert_eq!(config.preview(), "loading...");
    }
}
