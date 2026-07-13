//! Library-level state — books, settings, plugin configs.

use crate::library::Library;
use crate::settings::Settings;
use crate::state::settings_ui::SettingsUiState;
use crate::storage::manager::LibraryManager;

pub struct LibraryState {
    pub library: Library,
    pub manager: LibraryManager,
    pub settings: Settings,
    pub settings_ui: SettingsUiState,
}

impl LibraryState {
    pub fn new() -> Self {
        Self {
            library: Library::new(),
            manager: LibraryManager::new(vec![]),
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
