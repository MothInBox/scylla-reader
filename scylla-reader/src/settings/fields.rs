#[derive(PartialEq, Clone)]
pub enum SettingsField {
    Cookies,
    RateLimit,
    DebugLog,
    ReaderMode,
}

impl SettingsField {
    pub fn label(&self) -> &'static str {
        match self {
            SettingsField::Cookies => "Cookies",
            SettingsField::RateLimit => "Rate Limit (seconds between requests)",
            SettingsField::DebugLog => "Debug Logging",
            SettingsField::ReaderMode => "Reader Mode",
        }
    }

    pub fn all() -> Vec<SettingsField> {
        vec![
            SettingsField::Cookies,
            SettingsField::RateLimit,
            SettingsField::DebugLog,
            SettingsField::ReaderMode,
        ]
    }
}
