//! Settings field enum — typed, serializable list of configurable fields.

#[derive(PartialEq, Clone)]
pub enum SettingsField {
    RateLimit,
    DebugLog,
    ReaderMode,
    Plugins,
}

impl SettingsField {
    pub fn label(&self) -> &'static str {
        match self {
            SettingsField::RateLimit => "Rate Limit (seconds between requests)",
            SettingsField::DebugLog => "Debug Logging",
            SettingsField::ReaderMode => "Reader Mode",
            SettingsField::Plugins => "Plugin Configs",
        }
    }

    pub fn all() -> Vec<SettingsField> {
        vec![
            SettingsField::RateLimit,
            SettingsField::DebugLog,
            SettingsField::ReaderMode,
            SettingsField::Plugins,
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
