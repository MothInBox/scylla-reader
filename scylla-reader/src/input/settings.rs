use crate::input::keybinds::*;
use crate::messenger::AppCommand;
use crate::settings::{SettingsField, SettingsPage};
use crate::state::LibraryState;
use crossterm::event::{KeyCode, KeyEvent};

pub fn handle_settings(
    lib: &mut LibraryState,
    key: KeyEvent,
    cmd_tx: &std::sync::mpsc::Sender<AppCommand>,
) -> bool {
    match lib.settings_ui.settings_page.clone() {
        SettingsPage::Main => handle_settings_main(lib, key, cmd_tx),
        SettingsPage::DebugLog => handle_debug_log(lib, key),
        SettingsPage::PluginList => handle_plugin_list(lib, key),
        SettingsPage::PluginFields => handle_plugin_fields(lib, key),
        SettingsPage::PluginFieldEdit => handle_plugin_field_edit(lib, key),
    }
}

pub fn handle_settings_main(
    lib: &mut LibraryState,
    key: KeyEvent,
    cmd_tx: &std::sync::mpsc::Sender<AppCommand>,
) -> bool {
    let num_fields = SettingsField::all().len();
    match key.code {
        KEY_NAV_DOWN => {
            lib.settings_ui.selected_field =
                (lib.settings_ui.selected_field + 1).min(num_fields - 1);
            true
        }
        KEY_NAV_UP => {
            lib.settings_ui.selected_field = lib.settings_ui.selected_field.saturating_sub(1);
            true
        }
        KEY_ENTER => {
            match SettingsField::all()[lib.settings_ui.selected_field] {
                SettingsField::RateLimit => {
                    if lib.settings_ui.editing {
                        if let Ok(rate) = lib.settings_ui.edit_buffer.parse::<u64>() {
                            lib.settings.rate_limit_secs = rate;
                            lib.settings.save();
                            let _ = cmd_tx.send(AppCommand::SetRateLimit(rate));
                        }
                        lib.settings_ui.editing = false;
                    } else {
                        lib.settings_ui.edit_buffer = lib.settings.rate_limit_secs.to_string();
                        lib.settings_ui.editing = true;
                    }
                }
                SettingsField::DebugLog => {
                    lib.settings_ui.reload_log();
                    lib.settings_ui.log_scroll = 0;
                    lib.settings_ui.settings_page = SettingsPage::DebugLog;
                }
                SettingsField::ReaderMode => {
                    lib.settings.reader_mode = lib.settings.reader_mode.toggle();
                    lib.settings.save();
                }
                SettingsField::Plugins => {
                    lib.settings.reload_plugins();
                    lib.settings_ui.selected_plugin = 0;
                    lib.settings_ui.settings_page = SettingsPage::PluginList;
                }
            }
            true
        }
        KeyCode::Char(c) if lib.settings_ui.editing => {
            lib.settings_ui.edit_buffer.push(c);
            true
        }
        KEY_BACKSPACE if lib.settings_ui.editing => {
            lib.settings_ui.edit_buffer.pop();
            true
        }
        _ => true,
    }
}

fn handle_debug_log(lib: &mut LibraryState, key: KeyEvent) -> bool {
    match key.code {
        KEY_ENTER => {
            lib.settings.debug_log = !lib.settings.debug_log;
            crate::settings::set_debug(lib.settings.debug_log);
            lib.settings.save();
            if lib.settings.debug_log {
                let log_path = crate::settings::log_file();
                if let Some(parent) = log_path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::write(&log_path, "");
                crate::settings::log(
                    crate::settings::LogLevel::Debug,
                    "INPUT",
                    "Debug logging enabled",
                );
            }
            lib.settings_ui.reload_log();
            lib.settings_ui.log_scroll = 0;
            true
        }
        KEY_NAV_UP => {
            lib.settings_ui.log_scroll = lib.settings_ui.log_scroll.saturating_sub(1);
            true
        }
        KEY_NAV_DOWN => {
            let max = lib.settings_ui.log_lines.len().saturating_sub(1);
            lib.settings_ui.log_scroll = (lib.settings_ui.log_scroll + 1).min(max);
            true
        }
        _ => true,
    }
}

pub fn handle_plugin_list(lib: &mut LibraryState, key: KeyEvent) -> bool {
    let num_plugins = lib.settings.plugin_configs.len();
    match key.code {
        KEY_NAV_DOWN => {
            if num_plugins > 0 {
                lib.settings_ui.selected_plugin =
                    (lib.settings_ui.selected_plugin + 1).min(num_plugins - 1);
            }
            true
        }
        KEY_NAV_UP => {
            lib.settings_ui.selected_plugin = lib.settings_ui.selected_plugin.saturating_sub(1);
            true
        }
        KEY_ENTER => {
            if !lib.settings.plugin_configs.is_empty() {
                lib.settings_ui.selected_plugin_field = 0;
                lib.settings_ui.settings_page = SettingsPage::PluginFields;
            }
            true
        }
        _ => true,
    }
}

pub fn handle_plugin_fields(lib: &mut LibraryState, key: KeyEvent) -> bool {
    let Some(config) = lib
        .settings
        .plugin_configs
        .get(lib.settings_ui.selected_plugin)
    else {
        lib.settings_ui.settings_page = SettingsPage::PluginList;
        return true;
    };
    let cookie_extra = if config.accepts_cookies { 1 } else { 0 };
    let total = config.schema.len() + cookie_extra;
    match key.code {
        KEY_NAV_DOWN => {
            if total > 0 {
                lib.settings_ui.selected_plugin_field =
                    (lib.settings_ui.selected_plugin_field + 1).min(total - 1);
            }
            true
        }
        KEY_NAV_UP => {
            lib.settings_ui.selected_plugin_field =
                lib.settings_ui.selected_plugin_field.saturating_sub(1);
            true
        }
        KEY_ENTER => {
            let is_cookie = config.accepts_cookies
                && lib.settings_ui.selected_plugin_field == config.schema.len();
            if is_cookie {
                lib.settings_ui.plugin_field_buffer = config.cookies.clone();
            } else if let Some(field) = config.schema.get(lib.settings_ui.selected_plugin_field) {
                lib.settings_ui.plugin_field_buffer =
                    config.values.get(&field.key).cloned().unwrap_or_default();
            } else {
                return true;
            }
            lib.settings_ui.plugin_field_editing = true;
            lib.settings_ui.settings_page = SettingsPage::PluginFieldEdit;
            true
        }
        _ => true,
    }
}

pub fn handle_plugin_field_edit(lib: &mut LibraryState, key: KeyEvent) -> bool {
    match key.code {
        KEY_ENTER => {
            let result = lib.settings.save_current_field(
                lib.settings_ui.selected_plugin,
                lib.settings_ui.selected_plugin_field,
                &lib.settings_ui.plugin_field_buffer,
            );
            if let Err(e) = result {
                crate::settings::log(
                    crate::settings::LogLevel::Debug,
                    "INPUT",
                    &format!("Failed to save plugin field: {}", e),
                );
            }
            lib.settings_ui.plugin_field_buffer.clear();
            lib.settings_ui.plugin_field_editing = false;
            lib.settings_ui.settings_page = SettingsPage::PluginFields;
            true
        }
        KeyCode::Char(c) => {
            lib.settings_ui.plugin_field_buffer.push(c);
            true
        }
        KEY_BACKSPACE => {
            lib.settings_ui.plugin_field_buffer.pop();
            true
        }
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::ReaderMode;
    use crate::state::Page;
    use crate::test_helpers::*;
    use crossterm::event::KeyCode;

    #[test]
    fn test_handle_settings_main_navigate() {
        let mut state = test_state();
        let (tx, _rx) = channel();
        assert_eq!(state.lib.settings_ui.selected_field, 0);

        handle_settings_main(&mut state.lib, key_event(KEY_NAV_DOWN), &tx);
        assert_eq!(state.lib.settings_ui.selected_field, 1);

        handle_settings_main(&mut state.lib, key_event(KEY_NAV_DOWN), &tx);
        assert_eq!(state.lib.settings_ui.selected_field, 2);

        handle_settings_main(&mut state.lib, key_event(KEY_NAV_UP), &tx);
        assert_eq!(state.lib.settings_ui.selected_field, 1);

        handle_settings_main(&mut state.lib, key_event(KEY_NAV_UP), &tx);
        assert_eq!(state.lib.settings_ui.selected_field, 0);
    }

    #[test]
    fn test_handle_settings_main_tab_does_not_navigate() {
        let mut state = test_state();
        state.ui.page = Page::Settings;
        let (tx, _rx) = channel();
        let result = handle_settings_main(&mut state.lib, key_event(KeyCode::Tab), &tx);
        assert!(result);
    }

    #[test]
    fn test_handle_settings_main_enter_edits_rate_limit() {
        let mut state = test_state();
        state.lib.settings_ui.selected_field = 0;
        let (tx, _rx) = channel();
        assert!(!state.lib.settings_ui.editing);

        handle_settings_main(&mut state.lib, key_event(KEY_ENTER), &tx);
        assert!(state.lib.settings_ui.editing);
        assert_eq!(state.lib.settings_ui.edit_buffer, "2");
    }

    #[test]
    fn test_handle_settings_main_enter_saves_rate_limit() {
        let mut state = test_state();
        state.lib.settings_ui.selected_field = 0;
        state.lib.settings_ui.editing = true;
        state.lib.settings_ui.edit_buffer = "5".to_string();
        let (tx, _rx) = channel();

        let result = handle_settings_main(&mut state.lib, key_event(KEY_ENTER), &tx);
        assert!(result);
        assert!(!state.lib.settings_ui.editing);
        assert_eq!(state.lib.settings.rate_limit_secs, 5);
    }

    #[test]
    fn test_handle_settings_main_enter_debug_log() {
        let mut state = test_state();
        state.lib.settings_ui.selected_field = 1;
        let (tx, _rx) = channel();

        let result = handle_settings_main(&mut state.lib, key_event(KEY_ENTER), &tx);
        assert!(result);
        assert_eq!(state.lib.settings_ui.settings_page, SettingsPage::DebugLog);
    }

    #[test]
    fn test_handle_settings_main_enter_toggles_reader_mode() {
        let mut state = test_state();
        state.lib.settings_ui.selected_field = 2;
        let (tx, _rx) = channel();
        assert_eq!(state.lib.settings.reader_mode, ReaderMode::Paged);

        let result = handle_settings_main(&mut state.lib, key_event(KEY_ENTER), &tx);
        assert!(result);
        assert_eq!(state.lib.settings.reader_mode, ReaderMode::Scrollable);
    }

    #[test]
    fn test_handle_plugin_field_edit_enter_saves_and_goes_back() {
        let mut state = test_state();
        state.lib.settings_ui.settings_page = SettingsPage::PluginFieldEdit;
        let result = handle_plugin_field_edit(&mut state.lib, key_event(KEY_ENTER));
        assert!(result);
        assert_eq!(
            state.lib.settings_ui.settings_page,
            SettingsPage::PluginFields
        );
    }

    #[test]
    fn test_handle_settings_main_char_while_editing() {
        let mut state = test_state();
        state.lib.settings_ui.selected_field = 0;
        state.lib.settings_ui.editing = true;
        state.lib.settings_ui.edit_buffer = "2".to_string();
        let (tx, _rx) = channel();

        handle_settings_main(&mut state.lib, key_event(KeyCode::Char('5')), &tx);
        assert_eq!(state.lib.settings_ui.edit_buffer, "25");
    }

    #[test]
    fn test_handle_settings_main_backspace_while_editing() {
        let mut state = test_state();
        state.lib.settings_ui.selected_field = 0;
        state.lib.settings_ui.editing = true;
        state.lib.settings_ui.edit_buffer = "25".to_string();
        let (tx, _rx) = channel();

        handle_settings_main(&mut state.lib, key_event(KEY_BACKSPACE), &tx);
        assert_eq!(state.lib.settings_ui.edit_buffer, "2");
    }

    #[test]
    fn test_handle_settings_main_enter_invalid_rate_limit_keeps_old() {
        let mut state = test_state();
        state.lib.settings_ui.selected_field = 0;
        state.lib.settings_ui.editing = true;
        state.lib.settings_ui.edit_buffer = "not_a_number".to_string();
        let old_rate = state.lib.settings.rate_limit_secs;
        let (tx, _rx) = channel();

        let result = handle_settings_main(&mut state.lib, key_event(KEY_ENTER), &tx);
        assert!(result);
        assert!(!state.lib.settings_ui.editing);
        assert_eq!(state.lib.settings.rate_limit_secs, old_rate);
    }

    #[test]
    fn test_handle_settings_main_enter_toggles_reader_mode_and_saves() {
        let mut state = test_state();
        state.lib.settings_ui.selected_field = 2;
        let (tx, _rx) = channel();

        let result = handle_settings_main(&mut state.lib, key_event(KEY_ENTER), &tx);
        assert!(result);
        assert_eq!(state.lib.settings.reader_mode, ReaderMode::Scrollable);

        let result = handle_settings_main(&mut state.lib, key_event(KEY_ENTER), &tx);
        assert!(result);
        assert_eq!(state.lib.settings.reader_mode, ReaderMode::Paged);
    }

    #[test]
    fn test_handle_plugin_list_navigate() {
        let mut state = test_state();

        handle_plugin_list(&mut state.lib, key_event(KEY_NAV_DOWN));
        handle_plugin_list(&mut state.lib, key_event(KEY_NAV_UP));
        assert_eq!(state.lib.settings_ui.selected_plugin, 0);
    }
}
