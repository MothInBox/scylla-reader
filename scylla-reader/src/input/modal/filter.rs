//! Filter modal input — edit the library filter facets and commit on Enter.

use crate::input::keybinds::*;
use crate::library::{BookFilter, filter_tags, known_tags};
use crate::models::BookStatus;
use crate::state::modal::FilterRow;
use crate::state::{AppState, Modal};
use crossterm::event::{KeyCode, KeyEvent};
use scylla_core::messenger::AppCommand;

const STATUS_ROWS: [Option<BookStatus>; 5] = [
    None,
    Some(BookStatus::Reading),
    Some(BookStatus::Paused),
    Some(BookStatus::Dropped),
    Some(BookStatus::Completed),
];

pub fn handle_filter(
    state: &mut AppState,
    key: KeyEvent,
    _cmd_tx: &std::sync::mpsc::Sender<AppCommand>,
) -> bool {
    if !matches!(&state.ui.modal, Modal::Filter { .. }) {
        return true;
    }
    let focus = if let Modal::Filter { focus, .. } = &state.ui.modal {
        *focus
    } else {
        return true;
    };
    match key.code {
        KEY_ROW_NEXT => cycle_focus(state, 1),
        KEY_ROW_PREV => cycle_focus(state, -1),
        KEY_NAV_UP => move_cursor(state, -1),
        KEY_NAV_DOWN => move_cursor(state, 1),
        // `c` clears and space toggles only on choice rows; on Search they
        // must type into the name (multi-word names contain spaces and `c`).
        KEY_TOGGLE_ITEM if focus != FilterRow::Search => toggle_item(state),
        KEY_FILTER_CLEAR if focus != FilterRow::Search => clear_working(state),
        KEY_ENTER => commit(state),
        KEY_BACKSPACE => backspace(state),
        KeyCode::Char(c) => type_char(state, c),
        _ => {}
    }
    true
}

fn cycle_focus(state: &mut AppState, dir: i32) {
    if let Modal::Filter { focus, .. } = &mut state.ui.modal {
        const ROWS: [FilterRow; 5] = [
            FilterRow::Status,
            FilterRow::Search,
            FilterRow::Tags,
            FilterRow::Library,
            FilterRow::Ai,
        ];
        let idx = ROWS.iter().position(|r| r == focus).unwrap_or(0) as i32;
        let next = (idx + dir).rem_euclid(ROWS.len() as i32) as usize;
        *focus = ROWS[next];
    }
}

fn move_cursor(state: &mut AppState, dir: i32) {
    if let Modal::Filter {
        focus,
        tag_cursor,
        tag_query,
        status_cursor,
        lib_cursor,
        ..
    } = &mut state.ui.modal
    {
        match focus {
            FilterRow::Tags => {
                let tags = filter_tags(&known_tags(&state.lib.library.books), tag_query);
                let len = tags.len();
                if len > 0 {
                    if dir > 0 && *tag_cursor < len.saturating_sub(1) {
                        *tag_cursor += 1;
                    } else if dir < 0 && *tag_cursor > 0 {
                        *tag_cursor -= 1;
                    }
                }
            }
            FilterRow::Status => {
                let len = STATUS_ROWS.len();
                if dir > 0 && *status_cursor < len.saturating_sub(1) {
                    *status_cursor += 1;
                } else if dir < 0 && *status_cursor > 0 {
                    *status_cursor -= 1;
                }
            }
            FilterRow::Library => {
                let len = state.lib.manager.backend_names().len() + 1;
                if dir > 0 && *lib_cursor < len.saturating_sub(1) {
                    *lib_cursor += 1;
                } else if dir < 0 && *lib_cursor > 0 {
                    *lib_cursor -= 1;
                }
            }
            _ => {}
        }
    }
}

fn toggle_item(state: &mut AppState) {
    if let Modal::Filter {
        focus,
        working,
        tag_cursor,
        tag_query,
        status_cursor,
        lib_cursor,
        ..
    } = &mut state.ui.modal
    {
        match focus {
            FilterRow::Tags => {
                let tags = filter_tags(&known_tags(&state.lib.library.books), tag_query);
                if let Some(tag) = tags.get(*tag_cursor) {
                    if let Some(pos) = working.tags.iter().position(|t| t == tag) {
                        working.tags.remove(pos);
                    } else {
                        working.tags.push(tag.clone());
                    }
                }
            }
            FilterRow::Status => {
                working.status = STATUS_ROWS[*status_cursor].clone();
            }
            FilterRow::Library => {
                let names = state.lib.manager.backend_names();
                if *lib_cursor == 0 {
                    working.library = None;
                } else if let Some(name) = names.get(*lib_cursor - 1) {
                    working.library = Some(name.clone());
                }
            }
            _ => {}
        }
    }
}

fn type_char(state: &mut AppState, c: char) {
    if let Modal::Filter {
        focus,
        working,
        tag_query,
        ..
    } = &mut state.ui.modal
    {
        match focus {
            FilterRow::Search => working.name.push(c),
            FilterRow::Tags => tag_query.push(c),
            _ => {}
        }
    }
}

fn backspace(state: &mut AppState) {
    if let Modal::Filter {
        focus,
        working,
        tag_query,
        ..
    } = &mut state.ui.modal
    {
        match focus {
            FilterRow::Search => {
                working.name.pop();
            }
            FilterRow::Tags => {
                tag_query.pop();
            }
            _ => {}
        }
    }
}

fn clear_working(state: &mut AppState) {
    if let Modal::Filter { working, .. } = &mut state.ui.modal {
        working.name.clear();
        working.tags.clear();
        working.status = None;
    }
}

fn commit(state: &mut AppState) {
    let working: BookFilter = if let Modal::Filter { working, .. } = &state.ui.modal {
        working.clone()
    } else {
        return;
    };

    // Only reload from the backend when the library facet actually changed;
    // otherwise the in-memory list (with in-progress scraped books) is kept.
    let library_changed = working.library != state.lib.library.filter.library;
    state.lib.library.filter = working.clone();
    state.lib.library.selected_index = 0;
    state.lib.library.search_order = None;

    state
        .lib
        .manager
        .set_active_backend(working.library.clone());
    if library_changed {
        reload_books(state);
    }

    state.ui.modal = Modal::None;
}

/// Reload the book list from the active (or primary) backend. On failure the
/// current books are kept and the error is logged.
fn reload_books(state: &mut AppState) {
    if let Some(backend) = state.lib.manager.primary_backend() {
        match crate::storage::client::block_on(backend.list_books()) {
            Ok(books) => state.lib.library.books = books,
            Err(e) => crate::settings::log(
                crate::settings::LogLevel::Error,
                "FILTER",
                &format!("Failed to reload books: {}", e),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::*;
    use crossterm::event::KeyCode;

    fn filter_state() -> AppState {
        let mut state = test_state();
        state.ui.modal = Modal::Filter {
            working: BookFilter::default(),
            focus: FilterRow::Search,
            tag_query: String::new(),
            tag_cursor: 0,
            tag_scroll: 0,
            status_cursor: 0,
            lib_cursor: 0,
        };
        state
    }

    fn working(state: &AppState) -> BookFilter {
        if let Modal::Filter { working, .. } = &state.ui.modal {
            working.clone()
        } else {
            panic!("Expected Filter modal");
        }
    }

    fn focus(state: &AppState) -> FilterRow {
        if let Modal::Filter { focus, .. } = &state.ui.modal {
            *focus
        } else {
            panic!("Expected Filter modal");
        }
    }

    #[test]
    fn test_handle_filter_typing_edits_name_when_search_focused() {
        let mut state = filter_state();
        let (tx, _rx) = channel();
        handle_filter(&mut state, key_event(KeyCode::Char('g')), &tx);
        handle_filter(&mut state, key_event(KeyCode::Char('l')), &tx);
        handle_filter(&mut state, key_event(KeyCode::Char('o')), &tx);
        assert_eq!(working(&state).name, "glo");
    }

    #[test]
    fn test_handle_filter_backspace_edits_name_when_search_focused() {
        let mut state = filter_state();
        let (tx, _rx) = channel();
        handle_filter(&mut state, key_event(KeyCode::Char('g')), &tx);
        handle_filter(&mut state, key_event(KEY_BACKSPACE), &tx);
        assert_eq!(working(&state).name, "");
    }

    #[test]
    fn test_handle_filter_tab_cycles_focus() {
        let mut state = filter_state();
        let (tx, _rx) = channel();
        handle_filter(&mut state, key_event(KEY_ROW_NEXT), &tx);
        assert_eq!(focus(&state), FilterRow::Tags);
        handle_filter(&mut state, key_event(KEY_ROW_NEXT), &tx);
        assert_eq!(focus(&state), FilterRow::Library);
        handle_filter(&mut state, key_event(KEY_ROW_NEXT), &tx);
        assert_eq!(focus(&state), FilterRow::Ai);
        handle_filter(&mut state, key_event(KEY_ROW_NEXT), &tx);
        assert_eq!(focus(&state), FilterRow::Status);
    }

    #[test]
    fn test_handle_filter_backtab_cycles_focus_backwards() {
        let mut state = filter_state();
        let (tx, _rx) = channel();
        handle_filter(&mut state, key_event(KEY_ROW_PREV), &tx);
        assert_eq!(focus(&state), FilterRow::Status);
        handle_filter(&mut state, key_event(KEY_ROW_PREV), &tx);
        assert_eq!(focus(&state), FilterRow::Ai);
    }

    #[test]
    fn test_handle_filter_tag_query_narrows_candidates() {
        let mut state = filter_state();
        state.lib.library.add_book("B".into(), "u".into());
        state.lib.library.books[0].tags = vec!["fantasy".into(), "litrpg".into()];
        if let Modal::Filter {
            focus, tag_query, ..
        } = &mut state.ui.modal
        {
            *focus = FilterRow::Tags;
            tag_query.push_str("fan");
        }
        let (tx, _rx) = channel();
        handle_filter(&mut state, key_event(KeyCode::Char('t')), &tx);
        if let Modal::Filter { tag_query, .. } = &state.ui.modal {
            assert_eq!(tag_query, "fant");
        }
        // typing in Tags must not touch working.tags
        assert!(working(&state).tags.is_empty());
    }

    #[test]
    fn test_handle_filter_space_toggles_tag() {
        let mut state = filter_state();
        state.lib.library.add_book("B".into(), "u".into());
        state.lib.library.books[0].tags = vec!["fantasy".into(), "litrpg".into()];
        if let Modal::Filter { focus, .. } = &mut state.ui.modal {
            *focus = FilterRow::Tags;
        }
        let (tx, _rx) = channel();
        handle_filter(&mut state, key_event(KEY_TOGGLE_ITEM), &tx);
        assert_eq!(working(&state).tags, vec!["fantasy"]);
        handle_filter(&mut state, key_event(KEY_TOGGLE_ITEM), &tx);
        assert!(working(&state).tags.is_empty());
    }

    #[test]
    fn test_handle_filter_space_sets_status() {
        let mut state = filter_state();
        if let Modal::Filter {
            focus,
            status_cursor,
            ..
        } = &mut state.ui.modal
        {
            *focus = FilterRow::Status;
            *status_cursor = 2;
        }
        let (tx, _rx) = channel();
        handle_filter(&mut state, key_event(KEY_TOGGLE_ITEM), &tx);
        assert_eq!(working(&state).status, Some(BookStatus::Paused));
    }

    #[test]
    fn test_handle_filter_space_sets_status_any() {
        let mut state = filter_state();
        if let Modal::Filter {
            focus,
            working,
            status_cursor,
            ..
        } = &mut state.ui.modal
        {
            *focus = FilterRow::Status;
            working.status = Some(BookStatus::Reading);
            *status_cursor = 0;
        }
        let (tx, _rx) = channel();
        handle_filter(&mut state, key_event(KEY_TOGGLE_ITEM), &tx);
        assert_eq!(working(&state).status, None);
    }

    #[test]
    fn test_handle_filter_space_sets_library() {
        let mut state = test_state_with_backend(Box::new(MockBackend::new("remote1")));
        state.ui.modal = Modal::Filter {
            working: BookFilter::default(),
            focus: FilterRow::Library,
            tag_query: String::new(),
            tag_cursor: 0,
            tag_scroll: 0,
            status_cursor: 0,
            lib_cursor: 1,
        };
        let (tx, _rx) = channel();
        handle_filter(&mut state, key_event(KEY_TOGGLE_ITEM), &tx);
        assert_eq!(working(&state).library.as_deref(), Some("remote1"));
    }

    #[test]
    fn test_handle_filter_c_clears_facets_but_not_library() {
        let mut state = filter_state();
        if let Modal::Filter { focus, working, .. } = &mut state.ui.modal {
            *focus = FilterRow::Status;
            working.name = "glo".into();
            working.tags = vec!["fantasy".into()];
            working.status = Some(BookStatus::Reading);
            working.library = Some("remote1".into());
        }
        let (tx, _rx) = channel();
        handle_filter(&mut state, key_event(KEY_FILTER_CLEAR), &tx);
        let w = working(&state);
        assert!(w.name.is_empty());
        assert!(w.tags.is_empty());
        assert_eq!(w.status, None);
        assert_eq!(w.library.as_deref(), Some("remote1"));
    }

    #[test]
    fn test_handle_filter_c_types_when_search_focused() {
        let mut state = filter_state();
        let (tx, _rx) = channel();
        handle_filter(&mut state, key_event(KEY_FILTER_CLEAR), &tx);
        assert_eq!(working(&state).name, "c");
    }

    #[test]
    fn test_handle_filter_space_types_when_search_focused() {
        let mut state = filter_state();
        let (tx, _rx) = channel();
        handle_filter(&mut state, key_event(KEY_TOGGLE_ITEM), &tx);
        assert_eq!(working(&state).name, " ");
    }

    #[test]
    fn test_handle_filter_multi_word_name_with_c_and_space() {
        let mut state = filter_state();
        let (tx, _rx) = channel();
        for c in "dragon heart".chars() {
            handle_filter(&mut state, key_event(KeyCode::Char(c)), &tx);
        }
        assert_eq!(working(&state).name, "dragon heart");
    }

    #[test]
    fn test_handle_filter_enter_commits_and_closes() {
        let mut state = filter_state();
        if let Modal::Filter { working, .. } = &mut state.ui.modal {
            working.name = "glo".into();
        }
        state.lib.library.add_book("Gloom".into(), "u".into());
        state.lib.library.add_book("Other".into(), "u2".into());
        let (tx, _rx) = channel();
        handle_filter(&mut state, key_event(KEY_ENTER), &tx);
        assert_eq!(state.ui.modal, Modal::None);
        assert_eq!(state.lib.library.filter.name, "glo");
        assert_eq!(state.lib.library.selected_index, 0);
        assert!(state.lib.library.search_order.is_none());
        assert_eq!(state.lib.library.visible_indices(), vec![0]);
    }

    #[test]
    fn test_handle_filter_enter_applies_library_selection() {
        let mut state = test_state_with_backend(Box::new(MockBackend::new("remote1")));
        state.ui.modal = Modal::Filter {
            working: BookFilter {
                library: Some("remote1".into()),
                ..Default::default()
            },
            focus: FilterRow::Library,
            tag_query: String::new(),
            tag_cursor: 0,
            tag_scroll: 0,
            status_cursor: 0,
            lib_cursor: 1,
        };
        let (tx, _rx) = channel();
        handle_filter(&mut state, key_event(KEY_ENTER), &tx);
        assert_eq!(state.ui.modal, Modal::None);
        assert_eq!(
            state.lib.manager.active_filter,
            scylla_core::types::LibraryFilter::Backend("remote1".into())
        );
        assert_eq!(state.lib.library.filter.library.as_deref(), Some("remote1"));
    }

    #[test]
    fn test_handle_filter_enter_clears_library_selection() {
        let mut state = test_state_with_backend(Box::new(MockBackend::new("remote1")));
        state.lib.manager.set_active_backend(Some("remote1".into()));
        state.ui.modal = Modal::Filter {
            working: BookFilter::default(),
            focus: FilterRow::Library,
            tag_query: String::new(),
            tag_cursor: 0,
            tag_scroll: 0,
            status_cursor: 0,
            lib_cursor: 0,
        };
        let (tx, _rx) = channel();
        handle_filter(&mut state, key_event(KEY_ENTER), &tx);
        assert_eq!(
            state.lib.manager.active_filter,
            scylla_core::types::LibraryFilter::All
        );
        assert_eq!(state.lib.library.filter.library, None);
    }

    #[test]
    fn test_handle_filter_enter_reloads_books_from_backend() {
        let mock = MockBackend::new("remote1");
        mock.books.lock().unwrap().push(crate::models::Book {
            title: "From Backend".into(),
            url: "u".into(),
            status: BookStatus::Reading,
            sessions: vec![],
            active_session_id: None,
            tags: vec![],
            cover_url: None,
            description: None,
            chapters: vec![],
        });
        let mut state = test_state_with_backend(Box::new(mock));
        state.lib.library.add_book("Local".into(), "local".into());
        state.ui.modal = Modal::Filter {
            working: BookFilter {
                library: Some("remote1".into()),
                ..Default::default()
            },
            focus: FilterRow::Library,
            tag_query: String::new(),
            tag_cursor: 0,
            tag_scroll: 0,
            status_cursor: 0,
            lib_cursor: 1,
        };
        let (tx, _rx) = channel();
        handle_filter(&mut state, key_event(KEY_ENTER), &tx);
        assert_eq!(state.lib.library.books.len(), 1);
        assert_eq!(state.lib.library.books[0].title, "From Backend");
    }

    #[test]
    fn test_handle_filter_enter_keeps_books_on_reload_error() {
        let mock = MockBackend::new("remote1");
        mock.set_fail(true);
        let mut state = test_state_with_backend(Box::new(mock));
        state.lib.library.add_book("Local".into(), "local".into());
        state.ui.modal = Modal::Filter {
            working: BookFilter {
                library: Some("remote1".into()),
                ..Default::default()
            },
            focus: FilterRow::Library,
            tag_query: String::new(),
            tag_cursor: 0,
            tag_scroll: 0,
            status_cursor: 0,
            lib_cursor: 1,
        };
        let (tx, _rx) = channel();
        handle_filter(&mut state, key_event(KEY_ENTER), &tx);
        assert_eq!(state.lib.library.books.len(), 1);
        assert_eq!(state.lib.library.books[0].title, "Local");
    }

    #[test]
    fn test_handle_filter_enter_skips_reload_when_library_unchanged() {
        let mock = MockBackend::new("remote1");
        mock.books.lock().unwrap().push(crate::models::Book {
            title: "From Backend".into(),
            url: "u".into(),
            status: BookStatus::Reading,
            sessions: vec![],
            active_session_id: None,
            tags: vec![],
            cover_url: None,
            description: None,
            chapters: vec![],
        });
        let calls = mock.calls.clone();
        let mut state = test_state_with_backend(Box::new(mock));
        state.lib.library.add_book("Local".into(), "local".into());
        // working.library == filter.library (both None) → no reload.
        state.ui.modal = Modal::Filter {
            working: BookFilter::default(),
            focus: FilterRow::Search,
            tag_query: String::new(),
            tag_cursor: 0,
            tag_scroll: 0,
            status_cursor: 0,
            lib_cursor: 0,
        };
        let (tx, _rx) = channel();
        handle_filter(&mut state, key_event(KEY_ENTER), &tx);
        assert_eq!(state.lib.library.books.len(), 1);
        assert_eq!(state.lib.library.books[0].title, "Local");
        let calls = calls.lock().unwrap().clone();
        assert!(
            !calls.iter().any(|c| c == "list_books"),
            "expected no list_books call, got: {:?}",
            calls
        );
    }
}
