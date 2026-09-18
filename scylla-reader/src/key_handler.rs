use crate::input;
use crate::input::keybinds::*;
use crate::settings::SettingsPage;
use crate::state::{AppState, Modal, Page};
use crossterm::event::KeyEvent;
use ratatui::prelude::Rect;
use scylla_core::types::BookStatus;

pub fn handle_key(state: &mut AppState, key: KeyEvent, size: Rect) -> bool {
    // Capture the selected book before any input handling so we can persist
    // deletes/status changes made on either the page or modal (palette) path.
    let selected_before = state
        .lib
        .library
        .selected_book()
        .map(|b| (b.url.clone(), b.status.clone()));

    if state.ui.modal != Modal::None {
        if key.code == KEY_ESCAPE {
            state.close_modal();
            return true;
        }
        let handled = input::handle_input(state, key, size);
        sync_after_input(state, selected_before);
        return handled;
    }

    // While editing a settings value, route every key to the page handler so
    // global navigation keys type into the edit buffer instead of firing.
    if state.ui.page == Page::Settings && state.lib.settings_ui.editing {
        let handled = input::handle_input(state, key, size);
        sync_after_input(state, selected_before);
        return handled;
    }

    match key.code {
        KEY_LIBRARY => {
            crate::settings::log(crate::settings::LogLevel::Debug, "NAV", "Page: Library");
            state.ui.page = Page::Library;
            return true;
        }
        KEY_READER => {
            crate::settings::log(crate::settings::LogLevel::Debug, "NAV", "Page: Reader");
            state.ui.page = Page::Reader;
            let fetch = state.lib.library.selected_book().and_then(|book| {
                if state.reader.book_url == book.url {
                    return None;
                }
                let session = book
                    .active_session_id
                    .and_then(|id| book.sessions.iter().find(|s| s.id == id))
                    .or_else(|| book.sessions.first())?;
                let idx =
                    (session.progress.current as usize).min(book.chapters.len().saturating_sub(1));
                let ch = book.chapters.get(idx)?;
                Some((session.id, session.name.clone(), idx, ch.url.clone()))
            });
            if let Some((session_id, session_name, idx, url)) = fetch {
                state.reader.session_id = session_id;
                state.reader.session_name = session_name;
                state.reader.loading = true;
                let base = crate::storage::client::api_base(state);
                if let Err(e) = crate::storage::client::block_on(
                    crate::storage::client::enqueue_job(&base, "FetchChapter", &url, Some(idx)),
                ) {
                    crate::settings::log(
                        crate::settings::LogLevel::Error,
                        "UI",
                        &format!("Failed to enqueue chapter fetch: {}", e),
                    );
                    state.reader.loading = false;
                }
            }
            return true;
        }
        KEY_JOBS => {
            crate::settings::log(crate::settings::LogLevel::Debug, "NAV", "Page: Jobs");
            state.ui.page = Page::Jobs;
            return true;
        }
        KEY_SETTINGS => {
            crate::settings::log(crate::settings::LogLevel::Debug, "NAV", "Page: Settings");
            state.ui.page = Page::Settings;
            return true;
        }
        KEY_COMMAND_PALETTE => {
            let actions = crate::ui::palette::build_palette_actions();
            let filtered = crate::ui::palette::filter_actions(&actions, "");
            state.ui.modal = Modal::CommandPalette {
                query: String::new(),
                filtered,
                selected: 0,
            };
            return true;
        }
        KEY_TOGGLE_HINTS => {
            state.ui.show_hints = !state.ui.show_hints;
            return true;
        }
        KEY_ESCAPE => {
            if state.ui.page == Page::Settings {
                match state.lib.settings_ui.settings_page {
                    SettingsPage::Main => return false,
                    SettingsPage::DebugLog => {
                        state.lib.settings_ui.settings_page = SettingsPage::Main;
                    }
                    SettingsPage::PluginList => {
                        state.lib.settings_ui.settings_page = SettingsPage::Main;
                    }
                    SettingsPage::PluginFields => {
                        state.lib.settings_ui.settings_page = SettingsPage::PluginList;
                    }
                    SettingsPage::PluginFieldEdit => {
                        state.lib.settings_ui.plugin_field_buffer.clear();
                        state.lib.settings_ui.plugin_field_editing = false;
                        state.lib.settings_ui.settings_page = SettingsPage::PluginFields;
                    }
                    SettingsPage::Server => {
                        state.lib.settings_ui.settings_page = SettingsPage::Main;
                    }
                }
                return true;
            }
            return false;
        }
        _ => {}
    }

    let handled = input::handle_input(state, key, size);
    sync_after_input(state, selected_before);
    handled
}

/// Persist post-input changes to the primary backend: a delete when the
/// previously selected book vanished, and a status update when its status
/// changed. Failures are logged, never panicked.
fn sync_after_input(state: &mut AppState, selected_before: Option<(String, BookStatus)>) {
    if let Some((url, _)) = &selected_before
        && !state.lib.library.books.iter().any(|b| b.url == *url)
        && let Some(backend) = state.lib.manager.primary_backend()
        && let Err(e) = crate::storage::client::block_on(backend.delete_book(url))
    {
        crate::settings::log(
            crate::settings::LogLevel::Error,
            "UI",
            &format!("Failed to delete book {}: {}", url, e),
        );
    }

    if let Some((url, pre_status)) = &selected_before
        && let Some(book) = state.lib.library.books.iter().find(|b| b.url == *url)
        && book.status != *pre_status
        && let Some(backend) = state.lib.manager.primary_backend()
        && let Err(e) = crate::storage::client::block_on(backend.update_status(url, &book.status))
    {
        crate::settings::log(
            crate::settings::LogLevel::Error,
            "UI",
            &format!("Failed to update status for {}: {}", url, e),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Page;
    use crate::test_helpers::*;

    #[test]
    fn test_delete_book_persists_to_backend() {
        let mock = MockBackend::new("mock");
        let calls = mock.calls.clone();
        let mut state = test_state_with_backend(Box::new(mock));
        state
            .lib
            .library
            .add_book("Test".into(), "http://example.com/book".into());
        state.ui.page = Page::Library;
        handle_key(&mut state, key_event(KEY_DELETE), rect());
        assert!(state.lib.library.books.is_empty());
        let calls = calls.lock().unwrap().clone();
        assert!(
            calls.iter().any(|c| c == "delete:http://example.com/book"),
            "expected delete call, got: {:?}",
            calls
        );
    }

    #[test]
    fn test_cycle_status_persists_to_backend() {
        let mock = MockBackend::new("mock");
        let calls = mock.calls.clone();
        let mut state = test_state_with_backend(Box::new(mock));
        state
            .lib
            .library
            .add_book("Test".into(), "http://example.com/book".into());
        state.ui.page = Page::Library;
        handle_key(&mut state, key_event(KEY_CYCLE_STATUS), rect());
        assert_eq!(
            state.lib.library.books[0].status,
            scylla_core::types::BookStatus::Paused
        );
        let calls = calls.lock().unwrap().clone();
        assert!(
            calls
                .iter()
                .any(|c| c == "update_status:http://example.com/book:Paused"),
            "expected update_status call, got: {:?}",
            calls
        );
    }

    fn palette_state(mock: MockBackend, query: &str) -> AppState {
        let mut state = test_state_with_backend(Box::new(mock));
        state
            .lib
            .library
            .add_book("Test".into(), "http://example.com/book".into());
        state.ui.page = Page::Library;
        let actions = crate::ui::palette::build_palette_actions();
        let filtered = crate::ui::palette::filter_actions(&actions, query);
        state.ui.modal = Modal::CommandPalette {
            query: query.to_string(),
            filtered,
            selected: 0,
        };
        state
    }

    #[test]
    fn test_palette_delete_book_persists_to_backend() {
        let mock = MockBackend::new("mock");
        let calls = mock.calls.clone();
        let mut state = palette_state(mock, "Delete");
        handle_key(&mut state, key_event(KEY_ENTER), rect());
        assert!(state.lib.library.books.is_empty());
        let calls = calls.lock().unwrap().clone();
        assert!(
            calls.iter().any(|c| c == "delete:http://example.com/book"),
            "expected delete call, got: {:?}",
            calls
        );
    }

    #[test]
    fn test_palette_cycle_status_persists_to_backend() {
        let mock = MockBackend::new("mock");
        let calls = mock.calls.clone();
        let mut state = palette_state(mock, "Cycle Status");
        handle_key(&mut state, key_event(KEY_ENTER), rect());
        assert_eq!(
            state.lib.library.books[0].status,
            scylla_core::types::BookStatus::Paused
        );
        let calls = calls.lock().unwrap().clone();
        assert!(
            calls
                .iter()
                .any(|c| c == "update_status:http://example.com/book:Paused"),
            "expected update_status call, got: {:?}",
            calls
        );
    }

    #[test]
    fn test_settings_editing_suppresses_global_keys() {
        let mut state = test_state();
        state.ui.page = Page::Settings;
        state.lib.settings_ui.editing = true;
        state.lib.settings_ui.edit_buffer = "5".to_string();

        handle_key(&mut state, key_event(KEY_LIBRARY), rect());
        assert_eq!(state.ui.page, Page::Settings);
        assert_eq!(state.lib.settings_ui.edit_buffer, "51");
    }

    #[test]
    fn test_settings_not_editing_global_key_navigates() {
        let mut state = test_state();
        state.ui.page = Page::Settings;
        state.lib.settings_ui.editing = false;

        handle_key(&mut state, key_event(KEY_LIBRARY), rect());
        assert_eq!(state.ui.page, Page::Library);
    }
}
