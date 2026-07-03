use crate::settings::{SettingsField, SettingsPage};
use crate::state::{AppState, Page};
use crossterm::event::{KeyCode, KeyEvent};

pub fn handle_settings(state: &mut AppState, key: KeyEvent) -> bool {
    match state.settings.settings_page.clone() {
        SettingsPage::Main => handle_settings_main(state, key),
        SettingsPage::PluginList => handle_plugin_list(state, key),
        SettingsPage::PluginFields => handle_plugin_fields(state, key),
        SettingsPage::PluginFieldEdit => handle_plugin_field_edit(state, key),
    }
}

pub fn handle_settings_main(state: &mut AppState, key: KeyEvent) -> bool {
    let num_fields = SettingsField::all().len();
    match key.code {
        KeyCode::Tab | KeyCode::Esc => {
            state.current_page = Page::Library;
            true
        }
        KeyCode::Down => {
            state.settings.selected_field = (state.settings.selected_field + 1).min(num_fields - 1);
            true
        }
        KeyCode::Up => {
            state.settings.selected_field = state.settings.selected_field.saturating_sub(1);
            true
        }
        KeyCode::Enter => {
            match SettingsField::all()[state.settings.selected_field] {
                SettingsField::RateLimit => {
                    state.settings.edit_buffer = state.settings.rate_limit_secs.to_string();
                    state.settings.editing = true;
                }
                SettingsField::DebugLog => {
                    state.settings.debug_log = !state.settings.debug_log;
                    crate::settings::set_debug(state.settings.debug_log);
                    if state.settings.debug_log {
                        let _ = std::fs::write(crate::settings::LOG_FILE, "");
                        crate::settings::log_debug("Debug logging enabled");
                    }
                }
                SettingsField::ReaderMode => {
                    state.settings.reader_mode = state.settings.reader_mode.toggle();
                }
                SettingsField::Plugins => {
                    state.settings.reload_plugins();
                    state.settings.selected_plugin = 0;
                    state.settings.settings_page = SettingsPage::PluginList;
                }
            }
            true
        }
        KeyCode::Char(c) if state.settings.editing => {
            state.settings.edit_buffer.push(c);
            true
        }
        KeyCode::Backspace if state.settings.editing => {
            state.settings.edit_buffer.pop();
            true
        }
        _ => true,
    }
}

pub fn handle_plugin_list(state: &mut AppState, key: KeyEvent) -> bool {
    let num_plugins = state.settings.plugin_configs.len();
    match key.code {
        KeyCode::Esc => {
            state.settings.settings_page = SettingsPage::Main;
            true
        }
        KeyCode::Down => {
            if num_plugins > 0 {
                state.settings.selected_plugin =
                    (state.settings.selected_plugin + 1).min(num_plugins - 1);
            }
            true
        }
        KeyCode::Up => {
            state.settings.selected_plugin = state.settings.selected_plugin.saturating_sub(1);
            true
        }
        KeyCode::Enter => {
            if !state.settings.plugin_configs.is_empty() {
                state.settings.selected_plugin_field = 0;
                state.settings.settings_page = SettingsPage::PluginFields;
            }
            true
        }
        _ => true,
    }
}

pub fn handle_plugin_fields(state: &mut AppState, key: KeyEvent) -> bool {
    let Some(config) = state.settings.plugin_configs.get(state.settings.selected_plugin) else {
        state.settings.settings_page = SettingsPage::PluginList;
        return true;
    };
    let cookie_extra = if config.accepts_cookies { 1 } else { 0 };
    let total = config.schema.len() + cookie_extra;
    match key.code {
        KeyCode::Esc => {
            state.settings.settings_page = SettingsPage::PluginList;
            true
        }
        KeyCode::Down => {
            if total > 0 {
                state.settings.selected_plugin_field =
                    (state.settings.selected_plugin_field + 1).min(total - 1);
            }
            true
        }
        KeyCode::Up => {
            state.settings.selected_plugin_field =
                state.settings.selected_plugin_field.saturating_sub(1);
            true
        }
        KeyCode::Enter => {
            let is_cookie = config.accepts_cookies
                && state.settings.selected_plugin_field == config.schema.len();
            if is_cookie {
                state.settings.plugin_field_buffer = config.cookies.clone();
            } else if let Some(field) = config.schema.get(state.settings.selected_plugin_field) {
                state.settings.plugin_field_buffer = config
                    .values
                    .get(&field.key)
                    .cloned()
                    .unwrap_or_default();
            } else {
                return true;
            }
            state.settings.plugin_field_editing = true;
            state.settings.settings_page = SettingsPage::PluginFieldEdit;
            true
        }
        _ => true,
    }
}

pub fn handle_plugin_field_edit(state: &mut AppState, key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Esc => {
            state.settings.plugin_field_buffer.clear();
            state.settings.plugin_field_editing = false;
            state.settings.settings_page = SettingsPage::PluginFields;
            true
        }
        KeyCode::Enter => {
            let result = state.settings.save_current_field();
            if let Err(e) = result {
                crate::settings::log_debug(&format!("Failed to save plugin field: {}", e));
            }
            state.settings.plugin_field_buffer.clear();
            state.settings.plugin_field_editing = false;
            state.settings.settings_page = SettingsPage::PluginFields;
            true
        }
        KeyCode::Char(c) => {
            state.settings.plugin_field_buffer.push(c);
            true
        }
        KeyCode::Backspace => {
            state.settings.plugin_field_buffer.pop();
            true
        }
        _ => true,
    }
}
