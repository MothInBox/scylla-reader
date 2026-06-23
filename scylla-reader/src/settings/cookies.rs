use std::path::PathBuf;

pub fn config_dir() -> PathBuf {
    // Use home dir dynamically instead of hardcoding username
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

    /// Scan config dir for all *.txt files and return a CookieStore for each
    pub fn discover_all() -> Vec<CookieStore> {
        let dir = config_dir();
        let Ok(entries) = std::fs::read_dir(&dir) else {
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
        crate::settings::log_debug(&format!("Loading cookies from {:?}", self.path));
        let contents = std::fs::read_to_string(&self.path)
            .map_err(|e| format!(
                "Could not read cookie file at {:?}: {}",
                self.path, e
            ))?;

        let cookie_str: String = contents
            .lines()
            .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
            .collect::<Vec<_>>()
            .join("; ");

        if cookie_str.is_empty() {
            return Err(format!("Cookie file for '{}' is empty.", self.domain).into());
        }

        crate::settings::log_debug(&format!("Loaded {} chars for '{}'", cookie_str.len(), self.domain));
        Ok(cookie_str)
    }

    pub fn save(&self, contents: &str) -> Result<(), Box<dyn std::error::Error>> {
        std::fs::create_dir_all(self.path.parent().unwrap())?;
        std::fs::write(&self.path, contents)?;
        crate::settings::log_debug(&format!("Saved cookies to {:?}", self.path));
        Ok(())
    }

    pub fn load_raw(&self) -> String {
        std::fs::read_to_string(&self.path).unwrap_or_default()
    }

    pub fn preview(&self) -> String {
        let raw = self.load_raw();
        let raw = raw.trim();
        if raw.is_empty() {
            "<empty>".to_string()
        } else if raw.len() > 40 {
            format!("{}...", &raw[..40])
        } else {
            raw.to_string()
        }
    }
}
