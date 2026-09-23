//! Chapter-results modal input — navigate ranked books (collapsed) and their
//! chapters (expanded). Tab/Enter expand/collapse the focused book; when
//! expanded, Enter jumps to the focused chapter. `a` fetches the full per-book
//! chapter ranking (deeper drill-down) for the focused book.

use crate::event_types::{ChapterHit, SearchOutcome, ServerEvent};
use crate::input::keybinds::*;
use crate::state::modal::{FilterRow, SearchStatus};
use crate::state::{AppState, Modal, Page};
use crossterm::event::KeyEvent;

pub fn handle_chapter_results(state: &mut AppState, key: KeyEvent) -> bool {
    let (query, groups, cursor, expanded) = if let Modal::ChapterResults {
        query,
        groups,
        cursor,
        expanded,
        ..
    } = &state.ui.modal
    {
        (query.clone(), groups.clone(), *cursor, *expanded)
    } else {
        return true;
    };

    match key.code {
        KEY_DRILLDOWN_ALL => {
            // 'a': fetch the full per-book chapter ranking for the focused
            // book (chapter mode + book_url) and replace the modal content.
            let book_index = match expanded {
                None => cursor,
                Some(gi) => gi,
            };
            if let Some(book_url) = groups.get(book_index).map(|g| g.book_url.clone()) {
                start_drill_down(state, &query, &book_url);
            }
            true
        }
        KEY_ROW_NEXT => {
            // Tab: expand the focused group, or collapse back to books.
            if let Modal::ChapterResults {
                expanded: e,
                cursor: c,
                ..
            } = &mut state.ui.modal
            {
                match *e {
                    None => {
                        if groups.get(*c).is_some() {
                            *e = Some(*c);
                            *c = 0;
                        }
                    }
                    Some(gi) => {
                        *e = None;
                        *c = gi;
                    }
                }
            }
            true
        }
        KEY_ENTER => {
            match expanded {
                None => {
                    // Collapsed: expand the focused group, cursor → first chapter.
                    if groups.get(cursor).is_some()
                        && let Modal::ChapterResults {
                            expanded: e,
                            cursor: c,
                            ..
                        } = &mut state.ui.modal
                    {
                        *e = Some(cursor);
                        *c = 0;
                    }
                }
                Some(gi) => {
                    // Expanded: jump to the focused chapter.
                    let hit = groups[gi].chapters.get(cursor).cloned();
                    if let Some(hit) = hit {
                        jump_to_chapter(state, &hit);
                    }
                    state.ui.modal = Modal::None;
                }
            }
            true
        }
        KEY_NAV_UP => {
            if let Modal::ChapterResults {
                cursor: c,
                expanded: e,
                ..
            } = &mut state.ui.modal
            {
                match *e {
                    None => {
                        if *c > 0 {
                            *c -= 1;
                        }
                    }
                    Some(_) => {
                        if *c > 0 {
                            *c -= 1;
                        }
                    }
                }
            }
            true
        }
        KEY_NAV_DOWN => {
            if let Modal::ChapterResults {
                cursor: c,
                expanded: e,
                ..
            } = &mut state.ui.modal
            {
                match *e {
                    None => {
                        if *c + 1 < groups.len() {
                            *c += 1;
                        }
                    }
                    Some(gi) => {
                        if *c + 1 < groups[gi].chapters.len() {
                            *c += 1;
                        }
                    }
                }
            }
            true
        }
        KEY_ESCAPE => {
            state.ui.modal = Modal::None;
            true
        }
        KEY_REFINE => {
            // Reopen the filter modal seeded with the query, focused on Ai.
            let working = state.lib.library.filter.clone();
            state.ui.modal = Modal::Filter {
                working,
                focus: FilterRow::Ai,
                tag_query: String::new(),
                tag_cursor: 0,
                tag_scroll: 0,
                status_cursor: state.lib.library.filter.status_cursor(),
                lib_cursor: state
                    .lib
                    .library
                    .filter
                    .library_cursor(&state.lib.manager.backend_names()),
                ai_query: query,
            };
            true
        }
        _ => true,
    }
}

/// Fetch the full per-book chapter ranking for the drill-down modal. Sets the
/// modal to Loading, spawns a one-shot search thread (chapter mode + book_url),
/// and delivers the result via `ServerEvent::DrillDownResults`.
fn start_drill_down(state: &mut AppState, query: &str, book_url: &str) {
    crate::settings::log(
        crate::settings::LogLevel::Debug,
        "AI",
        &format!("Drill-down: all chapters for {} ({})", book_url, query),
    );
    if let Modal::ChapterResults { status: s, .. } = &mut state.ui.modal {
        *s = SearchStatus::Loading;
    }
    let base = crate::storage::client::api_base(state);
    let Some(tx) = crate::event_types::event_tx() else {
        crate::settings::log(
            crate::settings::LogLevel::Error,
            "AI",
            "No event channel — cannot run drill-down search",
        );
        return;
    };
    let thread_query = query.to_string();
    let thread_book_url = book_url.to_string();
    std::thread::spawn(move || {
        let result = crate::storage::client::block_on(crate::storage::client::search(
            &base,
            &thread_query,
            "chapter",
            Some(&thread_book_url),
            50,
        ));
        let outcome = match result {
            Ok(SearchOutcome::ChapterMode { hits, .. }) => Ok(hits),
            Ok(SearchOutcome::NoEmbeddings { .. }) => Ok(Vec::new()),
            Ok(SearchOutcome::BookMode { .. }) => Err("unexpected book-mode response".to_string()),
            Err(e) => Err(e),
        };
        let _ = tx.send(ServerEvent::DrillDownResults {
            book_url: thread_book_url,
            result: outcome,
        });
    });
}

/// Jump to the selected chapter: reuse the active/first session when the book
/// is in the library, otherwise use a guest session. Enqueues a FetchChapter
/// job and clears the reader spinner if the enqueue fails.
fn jump_to_chapter(state: &mut AppState, hit: &ChapterHit) {
    let book_url = hit.book_url.clone();
    let session = state
        .lib
        .library
        .books
        .iter()
        .find(|b| b.url == book_url)
        .and_then(|book| {
            book.active_session_id
                .and_then(|id| book.sessions.iter().find(|s| s.id == id))
                .or_else(|| book.sessions.first())
                .map(|s| (s.id, s.name.clone()))
        });

    match session {
        Some((session_id, session_name)) => {
            state.reader.session_id = session_id;
            state.reader.session_name = session_name;
            if let Some(book) = state
                .lib
                .library
                .books
                .iter_mut()
                .find(|b| b.url == book_url)
            {
                book.active_session_id = Some(session_id);
            }
            if let Some(backend) = state.lib.manager.primary_backend()
                && let Err(e) = crate::storage::client::block_on(
                    backend.set_active_session(&book_url, session_id),
                )
            {
                crate::settings::log(
                    crate::settings::LogLevel::Error,
                    "UI",
                    &format!("Failed to set active session: {}", e),
                );
            }
        }
        None => {
            // Book not in the library — guest session.
            state.reader.session_id = -1;
            state.reader.session_name = hit.book_title.clone();
        }
    }

    state.reader.loading = true;
    state.ui.page = Page::Reader;
    let base = crate::storage::client::api_base(state);
    if let Err(e) = crate::storage::client::block_on(crate::storage::client::enqueue_job(
        &base,
        "FetchChapter",
        &hit.chapter_url,
        Some(hit.chapter_idx),
    )) {
        crate::settings::log(
            crate::settings::LogLevel::Error,
            "UI",
            &format!("Failed to enqueue chapter fetch: {}", e),
        );
        state.reader.loading = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event_types::{ChapterGroup, ChapterHit, ServerEvent};
    use crate::state::modal::SearchStatus;
    use crate::test_helpers::*;
    use crossterm::event::KeyCode;

    fn hit(book_url: &str, book_title: &str, idx: usize, score: f32) -> ChapterHit {
        ChapterHit {
            book_url: book_url.into(),
            book_title: book_title.into(),
            chapter_url: format!("{}/ch{}", book_url, idx),
            chapter_idx: idx,
            chapter_title: format!("Ch{}", idx),
            score,
            genres: vec![],
        }
    }

    fn results_state() -> AppState {
        let mut state = test_state();
        state.ui.modal = Modal::ChapterResults {
            query: "dragon".into(),
            groups: vec![
                ChapterGroup {
                    book_url: "u1".into(),
                    book_title: "Book A".into(),
                    genres: vec!["Fantasy".into()],
                    chapters: vec![hit("u1", "Book A", 0, 90.0), hit("u1", "Book A", 1, 80.0)],
                    best_score: 90.0,
                },
                ChapterGroup {
                    book_url: "u2".into(),
                    book_title: "Book B".into(),
                    genres: vec![],
                    chapters: vec![hit("u2", "Book B", 0, 70.0)],
                    best_score: 70.0,
                },
            ],
            cursor: 0,
            scroll_offset: 0,
            status: SearchStatus::Ready,
            expanded: None,
        };
        state
    }

    fn modal_state(state: &AppState) -> (usize, Option<usize>) {
        if let Modal::ChapterResults {
            cursor, expanded, ..
        } = &state.ui.modal
        {
            (*cursor, *expanded)
        } else {
            panic!("Expected ChapterResults modal");
        }
    }

    /// Map the current cursor to the focused `(group_idx, chapter_idx)` at the
    /// current level. Collapsed: the cursor is a group index (chapter = None).
    /// Expanded: the cursor is a chapter index within the expanded group.
    fn focused_selection(
        groups: &[ChapterGroup],
        expanded: Option<usize>,
        cursor: usize,
    ) -> Option<(usize, Option<usize>)> {
        match expanded {
            None => groups.get(cursor).map(|_| (cursor, None)),
            Some(gi) => groups
                .get(gi)
                .and_then(|g| g.chapters.get(cursor).map(|_| (gi, Some(cursor)))),
        }
    }

    #[test]
    fn test_focused_selection_collapsed() {
        let groups = if let Modal::ChapterResults { groups, .. } = &results_state().ui.modal {
            groups.clone()
        } else {
            panic!("expected ChapterResults");
        };
        assert_eq!(focused_selection(&groups, None, 0), Some((0, None)));
        assert_eq!(focused_selection(&groups, None, 1), Some((1, None)));
        assert_eq!(focused_selection(&groups, None, 5), None);
    }

    #[test]
    fn test_focused_selection_expanded() {
        let groups = if let Modal::ChapterResults { groups, .. } = &results_state().ui.modal {
            groups.clone()
        } else {
            panic!("expected ChapterResults");
        };
        assert_eq!(focused_selection(&groups, Some(0), 1), Some((0, Some(1))));
        assert_eq!(focused_selection(&groups, Some(0), 5), None);
    }

    #[test]
    fn test_tab_expands_focused_group() {
        let mut state = results_state();
        handle_chapter_results(&mut state, key_event(KEY_ROW_NEXT));
        assert_eq!(modal_state(&state), (0, Some(0)));
    }

    #[test]
    fn test_tab_collapses_back_to_groups() {
        let mut state = results_state();
        if let Modal::ChapterResults {
            cursor, expanded, ..
        } = &mut state.ui.modal
        {
            *cursor = 1;
            *expanded = Some(1);
        }
        handle_chapter_results(&mut state, key_event(KEY_ROW_NEXT));
        // Collapse → cursor returns to the group index.
        assert_eq!(modal_state(&state), (1, None));
    }

    #[test]
    fn test_nav_down_collapsed_moves_across_groups() {
        let mut state = results_state();
        handle_chapter_results(&mut state, key_event(KEY_NAV_DOWN));
        assert_eq!(modal_state(&state), (1, None));
        // Clamps at the last group.
        handle_chapter_results(&mut state, key_event(KEY_NAV_DOWN));
        assert_eq!(modal_state(&state), (1, None));
    }

    #[test]
    fn test_nav_up_collapsed_clamps() {
        let mut state = results_state();
        if let Modal::ChapterResults { cursor, .. } = &mut state.ui.modal {
            *cursor = 1;
        }
        handle_chapter_results(&mut state, key_event(KEY_NAV_UP));
        assert_eq!(modal_state(&state), (0, None));
        handle_chapter_results(&mut state, key_event(KEY_NAV_UP));
        assert_eq!(modal_state(&state), (0, None));
    }

    #[test]
    fn test_nav_down_expanded_moves_across_chapters() {
        let mut state = results_state();
        if let Modal::ChapterResults {
            cursor: _,
            expanded,
            ..
        } = &mut state.ui.modal
        {
            *expanded = Some(0);
        }
        handle_chapter_results(&mut state, key_event(KEY_NAV_DOWN));
        assert_eq!(modal_state(&state), (1, Some(0)));
        // Clamps at the last chapter of the expanded group.
        handle_chapter_results(&mut state, key_event(KEY_NAV_DOWN));
        assert_eq!(modal_state(&state), (1, Some(0)));
    }

    #[test]
    fn test_nav_up_expanded_clamps() {
        let mut state = results_state();
        if let Modal::ChapterResults {
            cursor, expanded, ..
        } = &mut state.ui.modal
        {
            *cursor = 1;
            *expanded = Some(0);
        }
        handle_chapter_results(&mut state, key_event(KEY_NAV_UP));
        assert_eq!(modal_state(&state), (0, Some(0)));
        handle_chapter_results(&mut state, key_event(KEY_NAV_UP));
        assert_eq!(modal_state(&state), (0, Some(0)));
    }

    #[test]
    fn test_enter_expands_when_collapsed() {
        let mut state = results_state();
        handle_chapter_results(&mut state, key_event(KEY_ENTER));
        assert_eq!(modal_state(&state), (0, Some(0)));
        assert!(matches!(state.ui.modal, Modal::ChapterResults { .. }));
    }

    #[test]
    fn test_enter_jumps_when_expanded() {
        let mut state = results_state();
        if let Modal::ChapterResults {
            cursor: _,
            expanded,
            ..
        } = &mut state.ui.modal
        {
            *expanded = Some(0);
        }
        handle_chapter_results(&mut state, key_event(KEY_ENTER));
        assert_eq!(state.ui.modal, Modal::None);
        assert_eq!(state.ui.page, Page::Reader);
    }

    #[test]
    fn test_esc_closes_modal() {
        let mut state = results_state();
        handle_chapter_results(&mut state, key_event(KEY_ESCAPE));
        assert_eq!(state.ui.modal, Modal::None);
    }

    #[test]
    fn test_f_reopens_filter_seeded_with_query() {
        let mut state = results_state();
        handle_chapter_results(&mut state, key_event(KEY_REFINE));
        match &state.ui.modal {
            Modal::Filter {
                focus,
                ai_query,
                working,
                ..
            } => {
                assert_eq!(*focus, FilterRow::Ai);
                assert_eq!(ai_query, "dragon");
                assert_eq!(working, &state.lib.library.filter);
            }
            _ => panic!("Expected Filter modal"),
        }
    }

    #[test]
    fn test_enter_jumps_to_chapter_in_library_book() {
        let mock = MockBackend::new("mock");
        let calls = mock.calls.clone();
        let mut state = test_state_with_backend(Box::new(mock));
        state.lib.library.add_book("Book A".into(), "u1".into());
        if let Some(book) = state.lib.library.books.iter_mut().find(|b| b.url == "u1") {
            book.sessions.push(crate::models::Session {
                id: 7,
                book_url: "u1".into(),
                name: "S7".into(),
                progress: crate::models::Progress {
                    current: 0,
                    total: 10,
                },
                created_at: String::new(),
                updated_at: String::new(),
            });
            book.active_session_id = Some(7);
        }
        state.ui.modal = Modal::ChapterResults {
            query: "dragon".into(),
            groups: vec![ChapterGroup {
                book_url: "u1".into(),
                book_title: "Book A".into(),
                genres: vec![],
                chapters: vec![hit("u1", "Book A", 2, 90.0)],
                best_score: 90.0,
            }],
            cursor: 0,
            scroll_offset: 0,
            status: SearchStatus::Ready,
            expanded: Some(0),
        };
        handle_chapter_results(&mut state, key_event(KEY_ENTER));
        assert_eq!(state.ui.modal, Modal::None);
        assert_eq!(state.ui.page, Page::Reader);
        assert_eq!(state.reader.session_id, 7);
        assert_eq!(state.reader.session_name, "S7");
        // Enqueue fails in the test env → loading cleared.
        assert!(!state.reader.loading);
        let calls = calls.lock().unwrap().clone();
        assert!(
            calls.iter().any(|c| c == "set_active_session:u1:7"),
            "expected set_active_session call, got: {:?}",
            calls
        );
    }

    #[test]
    fn test_enter_jumps_to_chapter_guest_session() {
        // MockBackend's url() is "http://mock" (non-resolvable) so the
        // FetchChapter enqueue fails deterministically regardless of whether a
        // real server is running on 127.0.0.1:8080.
        let mut state = test_state_with_backend(Box::new(MockBackend::new("mock")));
        state.ui.modal = Modal::ChapterResults {
            query: "dragon".into(),
            groups: vec![ChapterGroup {
                book_url: "u-unknown".into(),
                book_title: "Remote Book".into(),
                genres: vec![],
                chapters: vec![hit("u-unknown", "Remote Book", 5, 90.0)],
                best_score: 90.0,
            }],
            cursor: 0,
            scroll_offset: 0,
            status: SearchStatus::Ready,
            expanded: Some(0),
        };
        handle_chapter_results(&mut state, key_event(KEY_ENTER));
        assert_eq!(state.ui.modal, Modal::None);
        assert_eq!(state.ui.page, Page::Reader);
        assert_eq!(state.reader.session_id, -1);
        assert_eq!(state.reader.session_name, "Remote Book");
        assert!(!state.reader.loading);
    }

    #[test]
    fn test_enter_with_no_groups_stays_open() {
        let mut state = test_state();
        state.ui.modal = Modal::ChapterResults {
            query: "dragon".into(),
            groups: vec![],
            cursor: 0,
            scroll_offset: 0,
            status: SearchStatus::Empty,
            expanded: None,
        };
        handle_chapter_results(&mut state, key_event(KEY_ENTER));
        assert!(matches!(state.ui.modal, Modal::ChapterResults { .. }));
    }

    #[test]
    fn test_unhandled_key_returns_true() {
        let mut state = results_state();
        let result = handle_chapter_results(&mut state, key_event(KeyCode::Char('x')));
        assert!(result);
    }

    #[test]
    fn test_a_fetches_full_drill_down() {
        let (tx, rx) = std::sync::mpsc::channel();
        crate::event_types::set_event_tx(tx);
        let mut state = results_state();
        handle_chapter_results(&mut state, key_event(KEY_DRILLDOWN_ALL));
        // Modal shows Loading while the fetch is in flight.
        if let Modal::ChapterResults { status, .. } = &state.ui.modal {
            assert_eq!(*status, SearchStatus::Loading);
        } else {
            panic!("expected ChapterResults");
        }
        // The search thread delivers a DrillDownResults event for the focused
        // book (u1); it fails fast in the test environment (no server) but the
        // event still arrives.
        let event = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        match event {
            ServerEvent::DrillDownResults { book_url, result } => {
                assert_eq!(book_url, "u1");
                assert!(
                    result.is_err(),
                    "expected Err in test env, got {:?}",
                    result
                );
            }
            _ => panic!("expected DrillDownResults"),
        }
    }
}
