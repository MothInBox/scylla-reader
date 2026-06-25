use crate::settings::{SettingsField, SettingsPage};
use crate::state::{AppState, Page};
use crossterm::event::{KeyCode, KeyEvent};

pub fn handle_settings(state: &mut AppState, key: KeyEvent) -> bool {
    match state.settings.settings_page.clone() {
        SettingsPage::Main => handle_settings_main(state, key),
        SettingsPage::CookieList => handle_cookie_list(state, key),
        SettingsPage::CookieEdit => handle_cookie_edit(state, key),
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
                SettingsField::Cookies => {
                    state.settings.reload_cookies();
                    state.settings.settings_page = SettingsPage::CookieList;
                }
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

pub fn handle_cookie_list(state: &mut AppState, key: KeyEvent) -> bool {
    let num_cookies = state.settings.cookie_stores.len();
    match key.code {
        KeyCode::Esc => {
            state.settings.settings_page = SettingsPage::Main;
            true
        }
        KeyCode::Down => {
            if num_cookies > 0 {
                state.settings.selected_cookie =
                    (state.settings.selected_cookie + 1).min(num_cookies - 1);
            }
            true
        }
        KeyCode::Up => {
            state.settings.selected_cookie = state.settings.selected_cookie.saturating_sub(1);
            true
        }
        KeyCode::Enter => {
            if let Some(store) = state
                .settings
                .cookie_stores
                .get(state.settings.selected_cookie)
            {
                state.settings.cookie_edit_buffer = store.load_raw();
                state.settings.settings_page = SettingsPage::CookieEdit;
            }
            true
        }
        _ => true,
    }
}

pub fn handle_cookie_edit(state: &mut AppState, key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Esc => {
            state.settings.cookie_edit_buffer.clear();
            state.settings.settings_page = SettingsPage::CookieList;
            true
        }
        KeyCode::Enter => {
            state.settings.save_current_cookie();
            state.settings.cookie_edit_buffer.clear();
            state.settings.settings_page = SettingsPage::CookieList;
            true
        }
        KeyCode::Char(c) => {
            state.settings.cookie_edit_buffer.push(c);
            true
        }
        KeyCode::Backspace => {
            state.settings.cookie_edit_buffer.pop();
            true
        }
        _ => true,
    }
}
