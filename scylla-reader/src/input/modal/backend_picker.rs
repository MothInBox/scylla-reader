//! Backend picker modal — manage library backends: add, remove, rename, select.

use crate::input::keybinds::*;
use crate::state::{AppState, Modal};
use crate::storage::manager::LibraryManager;
use crate::storage::remote::RemoteApi;
use crossterm::event::{KeyCode, KeyEvent};
use scylla_core::types::*;

pub fn handle_backend_picker(state: &mut AppState, key: KeyEvent) -> bool {
    let is_editing = if let Modal::BackendPicker { input, .. } = &state.ui.modal {
        input.is_some()
    } else {
        return true;
    };

    if is_editing {
        return handle_editing(state, key);
    }

    match key.code {
        KEY_NAV_UP => {
            if let Modal::BackendPicker { cursor, .. } = &mut state.ui.modal
                && *cursor > 0
            {
                *cursor -= 1;
            }
        }
        KEY_NAV_DOWN => {
            let count = state.lib.manager.backends.len();
            if let Modal::BackendPicker { cursor, .. } = &mut state.ui.modal
                && *cursor < count.saturating_sub(1)
            {
                *cursor += 1;
            }
        }
        KEY_ESCAPE => {
            state.ui.modal = Modal::None;
        }
        KEY_ENTER => {
            // Select backend as active filter
            let idx = if let Modal::BackendPicker { cursor, .. } = &state.ui.modal {
                *cursor
            } else {
                return true;
            };
            if let Some(name) = state
                .lib
                .manager
                .backends
                .get(idx)
                .map(|b| b.name().to_string())
            {
                state.lib.manager.set_active_backend(Some(name.clone()));
                state.lib.library.filter.library = Some(name.clone());
                state.lib.library.selected_index = 0;
                state.lib.library.search_order = None;
                if let Some(backend) = state.lib.manager.primary_backend() {
                    match crate::storage::client::block_on(backend.list_books()) {
                        Ok(books) => state.lib.library.books = books,
                        Err(e) => crate::settings::log(
                            crate::settings::LogLevel::Error,
                            "BACKEND",
                            &format!("Failed to reload books: {}", e),
                        ),
                    }
                }
            }
            state.ui.modal = Modal::None;
        }
        KEY_NEW_SESSION => {
            if let Modal::BackendPicker {
                cursor,
                input,
                editing_idx,
                pending_delete_idx,
                ..
            } = &mut state.ui.modal
            {
                *cursor = 0;
                *input = Some(String::new());
                *editing_idx = None;
                *pending_delete_idx = None;
            }
        }
        KEY_RENAME_SESSION => {
            if let Modal::BackendPicker {
                pending_delete_idx, ..
            } = &mut state.ui.modal
            {
                *pending_delete_idx = None;
            }
            let idx = if let Modal::BackendPicker { cursor, .. } = &state.ui.modal {
                *cursor
            } else {
                return true;
            };
            if let Some(backend) = state.lib.manager.backends.get(idx)
                && let Modal::BackendPicker {
                    input, editing_idx, ..
                } = &mut state.ui.modal
            {
                *input = Some(backend.name().to_string());
                *editing_idx = Some(idx);
            }
        }
        KEY_DELETE_SESSION => {
            let count = state.lib.manager.backends.len();
            let idx = if let Modal::BackendPicker { cursor, .. } = &state.ui.modal {
                *cursor
            } else {
                return true;
            };

            if idx < count {
                if count == 1
                    && let Modal::BackendPicker {
                        pending_delete_idx, ..
                    } = &mut state.ui.modal
                {
                    if pending_delete_idx.is_none() {
                        *pending_delete_idx = Some(idx);
                        return true;
                    }
                    *pending_delete_idx = None;
                }

                if count > 1 {
                    let deleted_name = state.lib.manager.backends[idx].name().to_string();
                    state.lib.manager.backends.remove(idx);
                    if state.lib.library.filter.library.as_deref() == Some(deleted_name.as_str()) {
                        state.lib.library.filter.library = None;
                        state.lib.library.selected_index = 0;
                    }
                    let new_cursor = idx.min(count.saturating_sub(2));
                    if let Modal::BackendPicker { cursor, .. } = &mut state.ui.modal {
                        *cursor = new_cursor;
                    }
                    save_backend_configs(&state.lib.manager);
                } else {
                    let deleted_name = state.lib.manager.backends[0].name().to_string();
                    state.lib.manager.backends.clear();
                    if state.lib.library.filter.library.as_deref() == Some(deleted_name.as_str()) {
                        state.lib.library.filter.library = None;
                        state.lib.library.selected_index = 0;
                    }
                    save_backend_configs(&state.lib.manager);
                    state.ui.modal = Modal::None;
                }
            }
        }
        _ => {
            if let Modal::BackendPicker {
                pending_delete_idx, ..
            } = &mut state.ui.modal
            {
                *pending_delete_idx = None;
            }
        }
    }

    true
}

/// Build persisted library configs from the manager's live backends,
/// preserving each backend's URL (empty when the backend has none).
fn backend_configs(manager: &LibraryManager) -> Vec<LibraryConfig> {
    manager
        .backends
        .iter()
        .map(|b| LibraryConfig {
            name: b.name().to_string(),
            url: b.url().unwrap_or_default(),
            default: false,
        })
        .collect()
}

/// Persist the manager's current backends to `libraries.json`.
fn save_backend_configs(manager: &LibraryManager) {
    if let Err(e) = crate::storage::config::save_libraries(&backend_configs(manager)) {
        crate::settings::log(
            crate::settings::LogLevel::Error,
            "CONFIG",
            &format!("Failed to save libraries config: {}", e),
        );
    }
}

/// Return `desired` unless a backend already uses it, appending a deterministic
/// numeric suffix (`name-2`, `name-3`, ...) to avoid name collisions.
fn unique_backend_name(manager: &LibraryManager, desired: &str) -> String {
    let taken = |candidate: &str| manager.backends.iter().any(|b| b.name() == candidate);
    if !taken(desired) {
        return desired.to_string();
    }
    let mut suffix = 2;
    loop {
        let candidate = format!("{desired}-{suffix}");
        if !taken(&candidate) {
            return candidate;
        }
        suffix += 1;
    }
}

/// Replace the backend at `idx` with a fresh remote backend that keeps the
/// original URL but uses `name`. The new name is deduped against the other
/// backends; renaming to the current name is a no-op.
fn rename_backend(manager: &mut LibraryManager, idx: usize, name: String) {
    let Some(url) = manager
        .backends
        .get(idx)
        .map(|b| b.url().unwrap_or_default())
    else {
        return;
    };
    if manager.backends[idx].name() == name {
        return;
    }
    let name = unique_backend_name(manager, &name);
    let backend = Box::new(RemoteApi::new(name, url));
    manager.backends.remove(idx);
    manager.backends.insert(idx, backend);
}

/// Append a backend for the entered value. URL-looking values become the base
/// URL with the display name defaulting to `remote`; otherwise the URL is
/// derived from the name. The resulting name is always made unique.
fn add_backend(manager: &mut LibraryManager, entered: &str) {
    let (desired_name, url) = if entered.starts_with("http://") || entered.starts_with("https://") {
        ("remote".to_string(), entered.to_string())
    } else {
        (entered.to_string(), format!("http://{}", entered))
    };
    let name = unique_backend_name(manager, &desired_name);
    manager.backends.push(Box::new(RemoteApi::new(name, url)));
}

fn handle_editing(state: &mut AppState, key: KeyEvent) -> bool {
    match key.code {
        KEY_ESCAPE => {
            if let Modal::BackendPicker {
                input, editing_idx, ..
            } = &mut state.ui.modal
            {
                *input = None;
                *editing_idx = None;
            }
        }
        KEY_ENTER => {
            let input_text = if let Modal::BackendPicker {
                input: Some(text), ..
            } = &state.ui.modal
            {
                text.clone()
            } else {
                return true;
            };

            let name = input_text.trim().to_string();
            if name.is_empty() {
                return true;
            }

            let editing_idx = if let Modal::BackendPicker { editing_idx, .. } = &state.ui.modal {
                *editing_idx
            } else {
                None
            };

            if let Some(idx) = editing_idx {
                rename_backend(&mut state.lib.manager, idx, name);
            } else {
                add_backend(&mut state.lib.manager, &name);
            }

            save_backend_configs(&state.lib.manager);

            if let Modal::BackendPicker {
                input,
                editing_idx,
                cursor,
                ..
            } = &mut state.ui.modal
            {
                *input = None;
                *editing_idx = None;
                *cursor = 0;
            }
        }
        KEY_BACKSPACE => {
            if let Modal::BackendPicker {
                input: Some(text), ..
            } = &mut state.ui.modal
            {
                text.pop();
            }
        }
        KeyCode::Char(c) => {
            if let Modal::BackendPicker {
                input: Some(text), ..
            } = &mut state.ui.modal
            {
                text.push(c);
            }
        }
        _ => {}
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::{
        MockBackend, key_event, test_state_with_backend, test_state_with_backends,
    };
    use crossterm::event::KeyCode;

    fn mock(name: &str) -> Box<dyn StorageBackend> {
        Box::new(MockBackend::new(name))
    }

    fn remote(name: &str, url: &str) -> Box<dyn StorageBackend> {
        Box::new(RemoteApi::new(name.to_string(), url.to_string()))
    }

    #[test]
    fn test_rename_backend_preserves_url() {
        let mut state = test_state_with_backend(mock("old"));
        rename_backend(&mut state.lib.manager, 0, "new".to_string());

        assert_eq!(state.lib.manager.backends.len(), 1);
        assert_eq!(state.lib.manager.backends[0].name(), "new");
        assert_eq!(
            state.lib.manager.backends[0].url(),
            Some("http://mock".to_string())
        );
    }

    #[test]
    fn test_rename_backend_out_of_range_is_noop() {
        let mut state = test_state_with_backend(mock("old"));
        rename_backend(&mut state.lib.manager, 5, "new".to_string());

        assert_eq!(state.lib.manager.backends.len(), 1);
        assert_eq!(state.lib.manager.backends[0].name(), "old");
    }

    #[test]
    fn test_rename_backend_dedupes_against_existing_names() {
        let mut state = test_state_with_backends(vec![mock("a"), mock("b")]);
        rename_backend(&mut state.lib.manager, 0, "b".to_string());

        let names: Vec<&str> = state
            .lib
            .manager
            .backends
            .iter()
            .map(|b| b.name())
            .collect();
        assert_eq!(names, vec!["b-2", "b"]);
    }

    #[test]
    fn test_rename_backend_to_same_name_is_noop() {
        let mut state = test_state_with_backend(mock("a"));
        rename_backend(&mut state.lib.manager, 0, "a".to_string());

        assert_eq!(state.lib.manager.backends.len(), 1);
        assert_eq!(state.lib.manager.backends[0].name(), "a");
    }

    #[test]
    fn test_whitespace_only_rename_is_rejected() {
        let mut state = test_state_with_backend(mock("old"));
        state.ui.modal = Modal::BackendPicker {
            cursor: 0,
            scroll_offset: 0,
            input: Some("   ".to_string()),
            editing_idx: Some(0),
            pending_delete_idx: None,
        };

        let handled = handle_editing(&mut state, key_event(KeyCode::Enter));
        assert!(handled);

        // Edit stays open and the backend is untouched.
        if let Modal::BackendPicker {
            input, editing_idx, ..
        } = &state.ui.modal
        {
            assert!(input.is_some());
            assert_eq!(*editing_idx, Some(0));
        } else {
            panic!("expected BackendPicker modal");
        }
        assert_eq!(state.lib.manager.backends[0].name(), "old");
    }

    #[test]
    fn test_deleted_backend_configs_preserve_urls() {
        let mut state = test_state_with_backends(vec![
            remote("a", "http://a.example"),
            remote("b", "http://b.example"),
        ]);
        state.lib.manager.backends.remove(0);

        let configs = backend_configs(&state.lib.manager);
        assert_eq!(configs.len(), 1);
        assert_eq!(configs[0].name, "b");
        assert_eq!(configs[0].url, "http://b.example");
    }

    #[test]
    fn test_backend_configs_preserve_all_urls() {
        let state = test_state_with_backends(vec![
            remote("a", "http://a.example"),
            remote("b", "https://b.example"),
        ]);

        let configs = backend_configs(&state.lib.manager);
        let urls: Vec<&str> = configs.iter().map(|c| c.url.as_str()).collect();
        assert_eq!(urls, vec!["http://a.example", "https://b.example"]);
    }

    #[test]
    fn test_add_backend_url_generates_unique_names() {
        let mut state = test_state_with_backend(mock("remote"));
        add_backend(&mut state.lib.manager, "http://example.com");
        add_backend(&mut state.lib.manager, "http://other.com");

        let names: Vec<&str> = state
            .lib
            .manager
            .backends
            .iter()
            .map(|b| b.name())
            .collect();
        assert_eq!(names, vec!["remote", "remote-2", "remote-3"]);
        assert_eq!(
            state.lib.manager.backends[1].url(),
            Some("http://example.com".to_string())
        );
        assert_eq!(
            state.lib.manager.backends[2].url(),
            Some("http://other.com".to_string())
        );
    }

    #[test]
    fn test_add_backend_plain_name_derives_url_and_unique_name() {
        let mut state = test_state_with_backend(mock("local"));
        add_backend(&mut state.lib.manager, "local");

        assert_eq!(state.lib.manager.backends[1].name(), "local-2");
        assert_eq!(
            state.lib.manager.backends[1].url(),
            Some("http://local".to_string())
        );
    }

    #[test]
    fn test_backend_configs_empty_after_clear() {
        let mut state = test_state_with_backend(mock("only"));
        state.lib.manager.backends.clear();

        assert!(backend_configs(&state.lib.manager).is_empty());
    }

    #[test]
    fn test_deleting_active_filter_backend_clears_filter_library() {
        crate::storage::config::set_config_dir_override(
            std::env::temp_dir().join(format!("scylla-test-config-{}", std::process::id())),
        );
        let mut state = test_state_with_backends(vec![mock("a"), mock("b")]);
        state.lib.library.filter.library = Some("a".into());
        state.lib.library.selected_index = 3;
        state.ui.modal = Modal::BackendPicker {
            cursor: 0,
            scroll_offset: 0,
            input: None,
            editing_idx: None,
            pending_delete_idx: None,
        };
        handle_backend_picker(&mut state, key_event(KEY_DELETE_SESSION));

        assert_eq!(state.lib.library.filter.library, None);
        assert_eq!(state.lib.library.selected_index, 0);
        assert_eq!(state.lib.manager.backends.len(), 1);
    }

    #[test]
    fn test_deleting_last_active_filter_backend_clears_filter_library() {
        crate::storage::config::set_config_dir_override(
            std::env::temp_dir().join(format!("scylla-test-config-{}", std::process::id())),
        );
        let mut state = test_state_with_backend(mock("only"));
        state.lib.library.filter.library = Some("only".into());
        state.lib.library.selected_index = 2;
        state.ui.modal = Modal::BackendPicker {
            cursor: 0,
            scroll_offset: 0,
            input: None,
            editing_idx: None,
            pending_delete_idx: None,
        };
        // First press arms the confirmation, second confirms the deletion.
        handle_backend_picker(&mut state, key_event(KEY_DELETE_SESSION));
        handle_backend_picker(&mut state, key_event(KEY_DELETE_SESSION));

        assert_eq!(state.lib.library.filter.library, None);
        assert_eq!(state.lib.library.selected_index, 0);
        assert!(state.lib.manager.backends.is_empty());
    }
}
