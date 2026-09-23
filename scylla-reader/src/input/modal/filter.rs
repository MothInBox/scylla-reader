//! Filter modal input — edit the library filter facets and commit on Enter.
//! The AI row runs a chapter-mode search (transient `ai_query`, not a facet).

use crate::event_types::ServerEvent;
use crate::input::keybinds::*;
use crate::library::{BookFilter, filter_tags, known_tags};
use crate::models::BookStatus;
use crate::state::modal::FilterRow;
use crate::state::{AppState, Modal};
use crossterm::event::{KeyCode, KeyEvent};

const STATUS_ROWS: [Option<BookStatus>; 5] = [
    None,
    Some(BookStatus::Reading),
    Some(BookStatus::Paused),
    Some(BookStatus::Dropped),
    Some(BookStatus::Completed),
];

pub fn handle_filter(state: &mut AppState, key: KeyEvent) -> bool {
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
        // `c` clears and space toggles only on choice rows; on Search and Ai
        // they must type into the text (multi-word queries contain spaces and
        // `c`).
        KEY_TOGGLE_ITEM if focus != FilterRow::Search && focus != FilterRow::Ai => {
            toggle_item(state)
        }
        KEY_FILTER_CLEAR if focus != FilterRow::Search && focus != FilterRow::Ai => {
            clear_working(state)
        }
        KEY_ENTER => {
            if focus == FilterRow::Ai {
                run_ai_search(state);
            } else {
                commit(state);
            }
        }
        KEY_BACKSPACE => backspace(state),
        KeyCode::Char(c) => type_char(state, c),
        _ => {}
    }
    true
}

/// Enter on the AI row: spawn a one-shot search thread (book mode — books with
/// inline top-3 chapters for instant drill-down) and activate the library AI
/// session. Empty queries are a no-op (stay in the modal).
fn run_ai_search(state: &mut AppState) {
    let query = if let Modal::Filter { ai_query, .. } = &state.ui.modal {
        ai_query.trim().to_string()
    } else {
        return;
    };
    if query.is_empty() {
        crate::settings::log(
            crate::settings::LogLevel::Debug,
            "AI",
            "Empty AI query — ignoring",
        );
        return;
    }
    crate::settings::log(
        crate::settings::LogLevel::Debug,
        "AI",
        &format!("Searching for: {}", query),
    );
    let base = crate::storage::client::api_base(state);
    let Some(tx) = crate::event_types::event_tx() else {
        crate::settings::log(
            crate::settings::LogLevel::Error,
            "AI",
            "No event channel — cannot run AI search",
        );
        return;
    };
    let thread_query = query.clone();
    std::thread::spawn(move || {
        let result = crate::storage::client::block_on(crate::storage::client::search(
            &base,
            &thread_query,
            "book",
            None,
            50,
        ));
        let _ = tx.send(ServerEvent::AiSearchResults {
            query: thread_query,
            result,
        });
    });
    // Activate the library AI session (Loading); results arrive via the event
    // channel. Close the filter modal so the re-ranked library is visible.
    state.lib.library.set_ai_searching(query);
    state.ui.modal = Modal::None;
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
        ai_query,
        ..
    } = &mut state.ui.modal
    {
        match focus {
            FilterRow::Search => working.name.push(c),
            FilterRow::Tags => tag_query.push(c),
            FilterRow::Ai => ai_query.push(c),
            _ => {}
        }
    }
}

fn backspace(state: &mut AppState) {
    if let Modal::Filter {
        focus,
        working,
        tag_query,
        ai_query,
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
            FilterRow::Ai => {
                ai_query.pop();
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
            ai_query: String::new(),
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
        handle_filter(&mut state, key_event(KeyCode::Char('g')));
        handle_filter(&mut state, key_event(KeyCode::Char('l')));
        handle_filter(&mut state, key_event(KeyCode::Char('o')));
        assert_eq!(working(&state).name, "glo");
    }

    #[test]
    fn test_handle_filter_backspace_edits_name_when_search_focused() {
        let mut state = filter_state();
        handle_filter(&mut state, key_event(KeyCode::Char('g')));
        handle_filter(&mut state, key_event(KEY_BACKSPACE));
        assert_eq!(working(&state).name, "");
    }

    #[test]
    fn test_handle_filter_tab_cycles_focus() {
        let mut state = filter_state();
        handle_filter(&mut state, key_event(KEY_ROW_NEXT));
        assert_eq!(focus(&state), FilterRow::Tags);
        handle_filter(&mut state, key_event(KEY_ROW_NEXT));
        assert_eq!(focus(&state), FilterRow::Library);
        handle_filter(&mut state, key_event(KEY_ROW_NEXT));
        assert_eq!(focus(&state), FilterRow::Ai);
        handle_filter(&mut state, key_event(KEY_ROW_NEXT));
        assert_eq!(focus(&state), FilterRow::Status);
    }

    #[test]
    fn test_handle_filter_backtab_cycles_focus_backwards() {
        let mut state = filter_state();
        handle_filter(&mut state, key_event(KEY_ROW_PREV));
        assert_eq!(focus(&state), FilterRow::Status);
        handle_filter(&mut state, key_event(KEY_ROW_PREV));
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
        handle_filter(&mut state, key_event(KeyCode::Char('t')));
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
        handle_filter(&mut state, key_event(KEY_TOGGLE_ITEM));
        assert_eq!(working(&state).tags, vec!["fantasy"]);
        handle_filter(&mut state, key_event(KEY_TOGGLE_ITEM));
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
        handle_filter(&mut state, key_event(KEY_TOGGLE_ITEM));
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
        handle_filter(&mut state, key_event(KEY_TOGGLE_ITEM));
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
            ai_query: String::new(),
        };
        handle_filter(&mut state, key_event(KEY_TOGGLE_ITEM));
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
        handle_filter(&mut state, key_event(KEY_FILTER_CLEAR));
        let w = working(&state);
        assert!(w.name.is_empty());
        assert!(w.tags.is_empty());
        assert_eq!(w.status, None);
        assert_eq!(w.library.as_deref(), Some("remote1"));
    }

    #[test]
    fn test_handle_filter_c_types_when_search_focused() {
        let mut state = filter_state();
        handle_filter(&mut state, key_event(KEY_FILTER_CLEAR));
        assert_eq!(working(&state).name, "c");
    }

    #[test]
    fn test_handle_filter_space_types_when_search_focused() {
        let mut state = filter_state();
        handle_filter(&mut state, key_event(KEY_TOGGLE_ITEM));
        assert_eq!(working(&state).name, " ");
    }

    #[test]
    fn test_handle_filter_multi_word_name_with_c_and_space() {
        let mut state = filter_state();
        for c in "dragon heart".chars() {
            handle_filter(&mut state, key_event(KeyCode::Char(c)));
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
        handle_filter(&mut state, key_event(KEY_ENTER));
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
            ai_query: String::new(),
        };
        handle_filter(&mut state, key_event(KEY_ENTER));
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
            ai_query: String::new(),
        };
        handle_filter(&mut state, key_event(KEY_ENTER));
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
            ai_query: String::new(),
        };
        handle_filter(&mut state, key_event(KEY_ENTER));
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
            ai_query: String::new(),
        };
        handle_filter(&mut state, key_event(KEY_ENTER));
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
            ai_query: String::new(),
        };
        handle_filter(&mut state, key_event(KEY_ENTER));
        assert_eq!(state.lib.library.books.len(), 1);
        assert_eq!(state.lib.library.books[0].title, "Local");
        let calls = calls.lock().unwrap().clone();
        assert!(
            !calls.iter().any(|c| c == "list_books"),
            "expected no list_books call, got: {:?}",
            calls
        );
    }

    fn ai_focus_state(query: &str) -> AppState {
        // MockBackend's url() is "http://mock" (non-resolvable) so the AI
        // search request fails deterministically regardless of whether a real
        // server is running on 127.0.0.1:8080.
        let mut state = test_state_with_backend(Box::new(MockBackend::new("mock")));
        state.ui.modal = Modal::Filter {
            working: BookFilter::default(),
            focus: FilterRow::Ai,
            tag_query: String::new(),
            tag_cursor: 0,
            tag_scroll: 0,
            status_cursor: 0,
            lib_cursor: 0,
            ai_query: query.to_string(),
        };
        state
    }

    fn ai_query(state: &AppState) -> String {
        if let Modal::Filter { ai_query, .. } = &state.ui.modal {
            ai_query.clone()
        } else {
            panic!("Expected Filter modal");
        }
    }

    #[test]
    fn test_handle_filter_typing_edits_ai_query_when_ai_focused() {
        let mut state = ai_focus_state("");
        handle_filter(&mut state, key_event(KeyCode::Char('d')));
        handle_filter(&mut state, key_event(KeyCode::Char('r')));
        handle_filter(&mut state, key_event(KeyCode::Char('a')));
        assert_eq!(ai_query(&state), "dra");
        // typing in Ai must not touch working.name
        assert!(working(&state).name.is_empty());
    }

    #[test]
    fn test_handle_filter_backspace_edits_ai_query_when_ai_focused() {
        let mut state = ai_focus_state("dragon");
        handle_filter(&mut state, key_event(KEY_BACKSPACE));
        assert_eq!(ai_query(&state), "drago");
    }

    #[test]
    fn test_handle_filter_c_types_when_ai_focused() {
        let mut state = ai_focus_state("");
        handle_filter(&mut state, key_event(KEY_FILTER_CLEAR));
        assert_eq!(ai_query(&state), "c");
    }

    #[test]
    fn test_handle_filter_space_types_when_ai_focused() {
        let mut state = ai_focus_state("");
        handle_filter(&mut state, key_event(KEY_TOGGLE_ITEM));
        assert_eq!(ai_query(&state), " ");
    }

    #[test]
    fn test_handle_filter_multi_word_ai_query_with_c_and_space() {
        let mut state = ai_focus_state("");
        for c in "dragon heart".chars() {
            handle_filter(&mut state, key_event(KeyCode::Char(c)));
        }
        assert_eq!(ai_query(&state), "dragon heart");
    }

    #[test]
    fn test_handle_filter_enter_on_ai_activates_library_ai() {
        let (tx, rx) = std::sync::mpsc::channel();
        crate::event_types::set_event_tx(tx);
        let mut state = ai_focus_state("dragon");
        handle_filter(&mut state, key_event(KEY_ENTER));
        // The filter modal closes and the library AI session is Loading.
        assert_eq!(state.ui.modal, Modal::None);
        let ai = state.lib.library.ai.as_ref().expect("AI session active");
        assert_eq!(ai.query, "dragon");
        assert_eq!(ai.status, crate::state::modal::SearchStatus::Loading);
        assert!(ai.book_mode);
        // The search thread delivers an AiSearchResults event (fails fast in
        // the test environment — no server — but the event still arrives).
        let event = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        match event {
            ServerEvent::AiSearchResults { query, result } => {
                assert_eq!(query, "dragon");
                assert!(
                    result.is_err(),
                    "expected Err in test env, got {:?}",
                    result
                );
            }
            _ => panic!("expected AiSearchResults"),
        }
    }

    #[test]
    fn test_handle_filter_enter_on_ai_empty_query_is_noop() {
        let mut state = ai_focus_state("   ");
        handle_filter(&mut state, key_event(KEY_ENTER));
        assert!(matches!(state.ui.modal, Modal::Filter { .. }));
    }
}
