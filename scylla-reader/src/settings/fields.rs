//! Settings field enum — typed, serializable list of configurable fields.

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_all_returns_four_variants() {
        let all = SettingsField::all();
        assert_eq!(all.len(), 4);
    }

    #[test]
    fn test_labels_are_non_empty() {
        for field in SettingsField::all() {
            assert!(!field.label().is_empty());
        }
    }
}
