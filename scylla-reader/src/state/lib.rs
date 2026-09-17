//! Library-level state — books, settings, plugin configs.

use crate::library::Library;
use crate::settings::Settings;
use crate::state::settings_ui::SettingsUiState;
use crate::storage::manager::LibraryManager;
use crate::storage::server_settings::ServerSettingsState;

pub struct LibraryState {
    pub library: Library,
    pub manager: LibraryManager,
    pub settings: Settings,
    pub settings_ui: SettingsUiState,
    pub server_settings: ServerSettingsState,
}

impl LibraryState {
    pub fn new() -> Self {
        Self {
            library: Library::new(),
            manager: LibraryManager::new(vec![]),
            settings: Settings::new(),
            settings_ui: SettingsUiState::new(),
            server_settings: ServerSettingsState::new(),
        }
    }
}

impl Default for LibraryState {
    fn default() -> Self {
        Self::new()
    }
}
