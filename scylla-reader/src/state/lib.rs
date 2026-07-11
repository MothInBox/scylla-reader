//! Library-level state — books, settings, plugin configs.

use crate::library::Library;
use crate::settings::Settings;
use crate::state::settings_ui::SettingsUiState;

pub struct LibraryState {
    pub library: Library,
    pub settings: Settings,
    pub settings_ui: SettingsUiState,
}

impl LibraryState {
    pub fn new() -> Self {
        Self {
            library: Library::new(),
            settings: Settings::new(),
            settings_ui: SettingsUiState::new(),
        }
    }
}

impl Default for LibraryState {
    fn default() -> Self {
        Self::new()
    }
}
