use std::path::PathBuf;

pub fn config_dir() -> PathBuf {
    PathBuf::from("/home/monkeinbox/.config/scylla-reader")
}

pub struct CookieStore {
    path: PathBuf,
}

impl CookieStore {
    pub fn path_buf(&self) -> &PathBuf {
        &self.path
    }
    pub fn for_domain(domain: &str) -> Self {
        let path = config_dir().join(format!("{}.txt", domain));
        Self { path }
    }

    pub fn load(&self) -> Result<String, Box<dyn std::error::Error>> {
        crate::settings::log_debug(&format!("Loading cookies from {:?}", self.path));
        let contents = std::fs::read_to_string(&self.path)
            .map_err(|e| format!(
                "Could not read cookie file at {:?}: {}\nRun: mkdir -p ~/.config/scylla-reader && touch {:?}",
                self.path, e, self.path
            ))?;

        let cookie_str: String = contents
            .lines()
            .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
            .collect::<Vec<_>>()
            .join("; ");

        crate::settings::log_debug(&format!("Loaded {} chars of cookies", cookie_str.len()));

        if cookie_str.is_empty() {
            return Err(format!(
                "Cookie file {:?} is empty. Paste your browser cookies into it.",
                self.path
            ).into());
        }

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
}
