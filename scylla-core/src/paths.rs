use std::path::PathBuf;
use std::sync::OnceLock;

pub static CONFIG_DIR_OVERRIDE: OnceLock<Option<PathBuf>> = OnceLock::new();
