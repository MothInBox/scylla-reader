use crate::settings::SettingsPage;

pub struct SettingsUiState {
    pub selected_field: usize,
    pub editing: bool,
    pub edit_buffer: String,
    pub settings_page: SettingsPage,
    pub selected_plugin: usize,
    pub selected_plugin_field: usize,
    pub plugin_field_editing: bool,
    pub plugin_field_buffer: String,
    pub log_scroll: usize,
    pub log_lines: Vec<String>,
}

impl Default for SettingsUiState {
    fn default() -> Self {
        Self::new()
    }
}

impl SettingsUiState {
    pub fn new() -> Self {
        Self {
            selected_field: 0,
            editing: false,
            edit_buffer: String::new(),
            settings_page: SettingsPage::Main,
            selected_plugin: 0,
            selected_plugin_field: 0,
            plugin_field_editing: false,
            plugin_field_buffer: String::new(),
            log_scroll: 0,
            log_lines: Vec::new(),
        }
    }

    pub fn reload_log(&mut self) {
        let path = crate::settings::log_file();
        let content = std::fs::read_to_string(&path).unwrap_or_default();
        self.log_lines = content.lines().map(|l| l.to_string()).collect();
    }
}
