use crate::messenger::AppCommand;
use crate::settings::{SettingsField, SettingsPage};
use crate::state::AppState;
use crossterm::event::{KeyCode, KeyEvent};

pub fn handle_settings(
    state: &mut AppState,
    key: KeyEvent,
    cmd_tx: &std::sync::mpsc::Sender<AppCommand>,
) -> bool {
    match state.settings.settings_page.clone() {
        SettingsPage::Main => handle_settings_main(state, key, cmd_tx),
        SettingsPage::DebugLog => handle_debug_log(state, key),
        SettingsPage::PluginList => handle_plugin_list(state, key),
        SettingsPage::PluginFields => handle_plugin_fields(state, key),
        SettingsPage::PluginFieldEdit => handle_plugin_field_edit(state, key),
    }
}

pub fn handle_settings_main(
    state: &mut AppState,
    key: KeyEvent,
    cmd_tx: &std::sync::mpsc::Sender<AppCommand>,
) -> bool {
    let num_fields = SettingsField::all().len();
    match key.code {
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
                    if state.settings.editing {
                        if let Ok(rate) = state.settings.edit_buffer.parse::<u64>() {
                            state.settings.rate_limit_secs = rate;
                            state.settings.save();
                            let _ = cmd_tx.send(AppCommand::SetRateLimit(rate));
                        }
                        state.settings.editing = false;
                    } else {
                        state.settings.edit_buffer = state.settings.rate_limit_secs.to_string();
                        state.settings.editing = true;
                    }
                }
                SettingsField::DebugLog => {
                    state.settings.reload_log();
                    state.settings.log_scroll = 0;
                    state.settings.settings_page = SettingsPage::DebugLog;
                }
                SettingsField::ReaderMode => {
                    state.settings.reader_mode = state.settings.reader_mode.toggle();
                    state.settings.save();
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

fn handle_debug_log(state: &mut AppState, key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Enter => {
            state.settings.debug_log = !state.settings.debug_log;
            crate::settings::set_debug(state.settings.debug_log);
            state.settings.save();
            if state.settings.debug_log {
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
            state.settings.reload_log();
            state.settings.log_scroll = 0;
            true
        }
        KeyCode::Up => {
            state.settings.log_scroll = state.settings.log_scroll.saturating_sub(1);
            true
        }
        KeyCode::Down => {
            let max = state.settings.log_lines.len().saturating_sub(1);
            state.settings.log_scroll = (state.settings.log_scroll + 1).min(max);
            true
        }
        _ => true,
    }
}

pub fn handle_plugin_list(state: &mut AppState, key: KeyEvent) -> bool {
    let num_plugins = state.settings.plugin_configs.len();
    match key.code {
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
    let Some(config) = state
        .settings
        .plugin_configs
        .get(state.settings.selected_plugin)
    else {
        state.settings.settings_page = SettingsPage::PluginList;
        return true;
    };
    let cookie_extra = if config.accepts_cookies { 1 } else { 0 };
    let total = config.schema.len() + cookie_extra;
    match key.code {
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
                state.settings.plugin_field_buffer =
                    config.values.get(&field.key).cloned().unwrap_or_default();
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
        KeyCode::Enter => {
            let result = state.settings.save_current_field();
            if let Err(e) = result {
                crate::settings::log(
                    crate::settings::LogLevel::Debug,
                    "INPUT",
                    &format!("Failed to save plugin field: {}", e),
                );
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;
    use crate::library::Library;
    use crate::settings::ReaderMode;
    use crate::state::Page;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn test_state() -> AppState {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let db = Db::open_conn(conn).unwrap();
        AppState::from_parts(db, Library::new())
    }

    fn key_event(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn channel() -> (
        std::sync::mpsc::Sender<AppCommand>,
        std::sync::mpsc::Receiver<AppCommand>,
    ) {
        std::sync::mpsc::channel()
    }

    #[test]
    fn test_handle_settings_main_navigate() {
        let mut state = test_state();
        let (tx, _rx) = channel();
        assert_eq!(state.settings.selected_field, 0);

        handle_settings_main(&mut state, key_event(KeyCode::Down), &tx);
        assert_eq!(state.settings.selected_field, 1);

        handle_settings_main(&mut state, key_event(KeyCode::Down), &tx);
        assert_eq!(state.settings.selected_field, 2);

        handle_settings_main(&mut state, key_event(KeyCode::Up), &tx);
        assert_eq!(state.settings.selected_field, 1);

        handle_settings_main(&mut state, key_event(KeyCode::Up), &tx);
        assert_eq!(state.settings.selected_field, 0);
    }

    #[test]
    fn test_handle_settings_main_tab_does_not_navigate() {
        let mut state = test_state();
        state.current_page = Page::Settings;
        let (tx, _rx) = channel();
        let result = handle_settings_main(&mut state, key_event(KeyCode::Tab), &tx);
        assert!(result);
        assert_eq!(state.current_page, Page::Settings);
    }

    #[test]
    fn test_handle_settings_main_enter_edits_rate_limit() {
        let mut state = test_state();
        state.settings.selected_field = 0;
        let (tx, _rx) = channel();
        assert!(!state.settings.editing);

        handle_settings_main(&mut state, key_event(KeyCode::Enter), &tx);
        assert!(state.settings.editing);
        assert_eq!(state.settings.edit_buffer, "2");
    }

    #[test]
    fn test_handle_settings_main_enter_saves_rate_limit() {
        let mut state = test_state();
        state.settings.selected_field = 0;
        state.settings.editing = true;
        state.settings.edit_buffer = "5".to_string();
        let (tx, _rx) = channel();

        let result = handle_settings_main(&mut state, key_event(KeyCode::Enter), &tx);
        assert!(result);
        assert!(!state.settings.editing);
        assert_eq!(state.settings.rate_limit_secs, 5);
    }

    #[test]
    fn test_handle_settings_main_enter_debug_log() {
        let mut state = test_state();
        state.settings.selected_field = 1;
        let (tx, _rx) = channel();

        let result = handle_settings_main(&mut state, key_event(KeyCode::Enter), &tx);
        assert!(result);
        assert_eq!(state.settings.settings_page, SettingsPage::DebugLog);
    }

    #[test]
    fn test_handle_settings_main_enter_toggles_reader_mode() {
        let mut state = test_state();
        state.settings.selected_field = 2;
        let (tx, _rx) = channel();
        assert_eq!(state.settings.reader_mode, ReaderMode::Paged);

        let result = handle_settings_main(&mut state, key_event(KeyCode::Enter), &tx);
        assert!(result);
        assert_eq!(state.settings.reader_mode, ReaderMode::Scrollable);
    }

    #[test]
    fn test_handle_plugin_field_edit_enter_saves_and_goes_back() {
        let mut state = test_state();
        state.settings.settings_page = SettingsPage::PluginFieldEdit;
        let result = handle_plugin_field_edit(&mut state, key_event(KeyCode::Enter));
        assert!(result);
        assert_eq!(state.settings.settings_page, SettingsPage::PluginFields);
    }

    #[test]
    fn test_handle_settings_main_char_while_editing() {
        let mut state = test_state();
        state.settings.selected_field = 0;
        state.settings.editing = true;
        state.settings.edit_buffer = "2".to_string();
        let (tx, _rx) = channel();

        handle_settings_main(&mut state, key_event(KeyCode::Char('5')), &tx);
        assert_eq!(state.settings.edit_buffer, "25");
    }

    #[test]
    fn test_handle_settings_main_backspace_while_editing() {
        let mut state = test_state();
        state.settings.selected_field = 0;
        state.settings.editing = true;
        state.settings.edit_buffer = "25".to_string();
        let (tx, _rx) = channel();

        handle_settings_main(&mut state, key_event(KeyCode::Backspace), &tx);
        assert_eq!(state.settings.edit_buffer, "2");
    }

    #[test]
    fn test_handle_settings_main_enter_invalid_rate_limit_keeps_old() {
        let mut state = test_state();
        state.settings.selected_field = 0;
        state.settings.editing = true;
        state.settings.edit_buffer = "not_a_number".to_string();
        let old_rate = state.settings.rate_limit_secs;
        let (tx, _rx) = channel();

        let result = handle_settings_main(&mut state, key_event(KeyCode::Enter), &tx);
        assert!(result);
        assert!(!state.settings.editing);
        assert_eq!(state.settings.rate_limit_secs, old_rate);
    }

    #[test]
    fn test_handle_settings_main_enter_toggles_reader_mode_and_saves() {
        let mut state = test_state();
        state.settings.selected_field = 2;
        let (tx, _rx) = channel();

        let result = handle_settings_main(&mut state, key_event(KeyCode::Enter), &tx);
        assert!(result);
        assert_eq!(state.settings.reader_mode, ReaderMode::Scrollable);

        let result = handle_settings_main(&mut state, key_event(KeyCode::Enter), &tx);
        assert!(result);
        assert_eq!(state.settings.reader_mode, ReaderMode::Paged);
    }

    #[test]
    fn test_handle_plugin_list_navigate() {
        let mut state = test_state();

        handle_plugin_list(&mut state, key_event(KeyCode::Down));
        handle_plugin_list(&mut state, key_event(KeyCode::Up));
        assert_eq!(state.settings.selected_plugin, 0);
    }
}
