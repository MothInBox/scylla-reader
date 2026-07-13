use std::sync::atomic::{AtomicBool, Ordering};

static ENABLED: AtomicBool = AtomicBool::new(false);

pub fn set_enabled(enabled: bool) {
    ENABLED.store(enabled, Ordering::Relaxed);
}

pub fn log(_level: &str, _tag: &str, _msg: &str) {
    if ENABLED.load(Ordering::Relaxed) {
        eprintln!("[{}][{}] {}", _level, _tag, _msg);
    }
}
