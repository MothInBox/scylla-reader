use std::fs;
use std::path::{Path, PathBuf};

pub fn config_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    PathBuf::from(home).join(".config/scylla-reader")
}

pub struct CookieStore {
    pub domain: String,
    path: PathBuf,
}

impl CookieStore {
    pub fn for_domain(domain: &str) -> Self {
        let path = config_dir().join(format!("{}.txt", domain));
        Self {
            domain: domain.to_string(),
            path,
        }
    }

    pub fn discover_all() -> Vec<CookieStore> {
        let dir = config_dir();
        let Ok(entries) = fs::read_dir(&dir) else {
            return vec![];
        };

        entries
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().map(|x| x == "txt").unwrap_or(false))
            .filter_map(|e| {
                let stem = e.path().file_stem()?.to_string_lossy().to_string();
                Some(CookieStore::for_domain(&stem))
            })
            .collect()
    }

    pub fn load(&self) -> Result<String, Box<dyn std::error::Error>> {
        let contents = fs::read_to_string(&self.path)
            .map_err(|e| format!("Could not read cookie file at {:?}: {}", self.path, e))?;

        let cookie_str: String = contents
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .collect::<Vec<_>>()
            .join("; ");

        if cookie_str.is_empty() {
            return Err(format!("Cookie file for '{}' is empty.", self.domain).into());
        }

        Ok(cookie_str)
    }

    pub fn save(&self, contents: &str) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&self.path, contents)?;
        Ok(())
    }

    pub fn load_raw(&self) -> String {
        fs::read_to_string(&self.path).unwrap_or_default()
    }

    pub fn preview(&self) -> String {
        let raw = self.load_raw();
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            "<empty>".to_string()
        } else if trimmed.len() > 40 {
            format!("{}...", &trimmed[..40])
        } else {
            trimmed.to_string()
        }
    }
    pub fn path(&self) -> &PathBuf {
        &self.path
    }
}
