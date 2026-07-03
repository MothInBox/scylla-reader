//! Per-domain cookie file management — load, save, preview, and discover.

use std::fs;
use std::path::PathBuf;

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
        } else if trimmed.chars().count() > 40 {
            format!("{}...", trimmed.chars().take(40).collect::<String>())
        } else {
            trimmed.to_string()
        }
    }
    pub fn path(&self) -> &PathBuf {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_dir_ends_with_scylla_reader() {
        let dir = config_dir();
        assert!(dir.to_string_lossy().ends_with("scylla-reader"));
    }

    #[test]
    fn test_for_domain_sets_domain_and_path() {
        let store = CookieStore::for_domain("example.com");
        assert_eq!(store.domain, "example.com");
        let path_str = store.path.to_string_lossy();
        assert!(path_str.contains("example.com.txt"));
    }

    #[test]
    fn test_preview_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.txt");
        let store = CookieStore {
            domain: "test".into(),
            path: path.clone(),
        };
        assert_eq!(store.preview(), "<empty>");
    }

    #[test]
    fn test_preview_short_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("short.txt");
        std::fs::write(&path, "hello world").unwrap();
        let store = CookieStore {
            domain: "test".into(),
            path,
        };
        assert_eq!(store.preview(), "hello world");
    }

    #[test]
    fn test_preview_long_content_ascii() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("long.txt");
        std::fs::write(&path, "a".repeat(100)).unwrap();
        let store = CookieStore {
            domain: "test".into(),
            path,
        };
        let preview = store.preview();
        assert_eq!(preview.len(), 43); // 40 chars + "..."
        assert!(preview.ends_with("..."));
    }

    #[test]
    fn test_preview_unicode_no_panic() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("unicode.txt");
        let content = "é".repeat(30);
        std::fs::write(&path, &content).unwrap();
        let store = CookieStore {
            domain: "test".into(),
            path,
        };
        let preview = store.preview();
        assert!(!preview.is_empty());
        assert!(preview.chars().count() <= 43); // 40 chars + "..."
    }

    #[test]
    fn test_save_and_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cookies.txt");
        let store = CookieStore {
            domain: "test".into(),
            path: path.clone(),
        };
        store.save("key=value\n# comment\nfoo=bar").unwrap();
        let loaded = store.load().unwrap();
        assert_eq!(loaded, "key=value; foo=bar");
    }

    #[test]
    fn test_load_empty_file_returns_err() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.txt");
        std::fs::write(&path, "").unwrap();
        let store = CookieStore {
            domain: "test".into(),
            path,
        };
        assert!(store.load().is_err());
    }

    #[test]
    fn test_load_comments_only_returns_err() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("comments.txt");
        std::fs::write(&path, "# just a comment\n# another\n").unwrap();
        let store = CookieStore {
            domain: "test".into(),
            path,
        };
        assert!(store.load().is_err());
    }

    #[test]
    fn test_discover_all_empty_dir() {
        let stores = CookieStore::discover_all();
        stores.iter().for_each(|_| {});
    }
}
