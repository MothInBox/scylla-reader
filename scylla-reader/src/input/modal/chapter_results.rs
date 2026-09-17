//! Chapter-results modal input — navigate grouped AI search hits and jump to a
//! chapter. Book headers are non-selectable; the cursor moves across the
//! flattened chapter rows.

use crate::event_types::{ChapterGroup, ChapterHit};
use crate::input::keybinds::*;
use crate::state::modal::FilterRow;
use crate::state::{AppState, Modal, Page};
use crossterm::event::KeyEvent;

pub fn handle_chapter_results(state: &mut AppState, key: KeyEvent) -> bool {
    let (query, groups, cursor) = if let Modal::ChapterResults {
        query,
        groups,
        cursor,
        ..
    } = &state.ui.modal
    {
        (query.clone(), groups.clone(), *cursor)
    } else {
        return true;
    };

    match key.code {
        KEY_NAV_UP => {
            if cursor > 0
                && let Modal::ChapterResults { cursor: c, .. } = &mut state.ui.modal
            {
                *c = cursor - 1;
            }
            true
        }
        KEY_NAV_DOWN => {
            let flat = flat_chapters(&groups);
            if cursor + 1 < flat.len()
                && let Modal::ChapterResults { cursor: c, .. } = &mut state.ui.modal
            {
                *c = cursor + 1;
            }
            true
        }
        KEY_ENTER => {
            let flat = flat_chapters(&groups);
            if let Some((group_idx, chapter_idx)) = flat.get(cursor) {
                let hit = &groups[*group_idx].chapters[*chapter_idx];
                jump_to_chapter(state, hit);
            }
            state.ui.modal = Modal::None;
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

/// Flatten the grouped chapters into `(group_idx, chapter_idx)` pairs — the
/// selectable rows (book headers are skipped).
fn flat_chapters(groups: &[ChapterGroup]) -> Vec<(usize, usize)> {
    let mut flat = Vec::new();
    for (gi, group) in groups.iter().enumerate() {
        for ci in 0..group.chapters.len() {
            flat.push((gi, ci));
        }
    }
    flat
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
    use crate::event_types::ChapterHit;
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
                    chapters: vec![hit("u1", "Book A", 0, 0.9), hit("u1", "Book A", 1, 0.8)],
                    best_score: 0.9,
                },
                ChapterGroup {
                    book_url: "u2".into(),
                    book_title: "Book B".into(),
                    genres: vec![],
                    chapters: vec![hit("u2", "Book B", 0, 0.7)],
                    best_score: 0.7,
                },
            ],
            cursor: 0,
            scroll_offset: 0,
            status: SearchStatus::Ready,
        };
        state
    }

    #[test]
    fn test_flat_chapters_skips_headers() {
        let groups = if let Modal::ChapterResults { groups, .. } = &results_state().ui.modal {
            groups.clone()
        } else {
            panic!("expected ChapterResults");
        };
        let flat = flat_chapters(&groups);
        assert_eq!(flat, vec![(0, 0), (0, 1), (1, 0)]);
    }

    #[test]
    fn test_nav_down_moves_across_flattened_rows() {
        let mut state = results_state();
        handle_chapter_results(&mut state, key_event(KEY_NAV_DOWN));
        if let Modal::ChapterResults { cursor, .. } = &state.ui.modal {
            assert_eq!(*cursor, 1);
        }
        handle_chapter_results(&mut state, key_event(KEY_NAV_DOWN));
        if let Modal::ChapterResults { cursor, .. } = &state.ui.modal {
            assert_eq!(*cursor, 2);
        }
        // Clamps at the last chapter.
        handle_chapter_results(&mut state, key_event(KEY_NAV_DOWN));
        if let Modal::ChapterResults { cursor, .. } = &state.ui.modal {
            assert_eq!(*cursor, 2);
        }
    }

    #[test]
    fn test_nav_up_moves_and_clamps() {
        let mut state = results_state();
        if let Modal::ChapterResults { cursor, .. } = &mut state.ui.modal {
            *cursor = 2;
        }
        handle_chapter_results(&mut state, key_event(KEY_NAV_UP));
        if let Modal::ChapterResults { cursor, .. } = &state.ui.modal {
            assert_eq!(*cursor, 1);
        }
        handle_chapter_results(&mut state, key_event(KEY_NAV_UP));
        handle_chapter_results(&mut state, key_event(KEY_NAV_UP));
        if let Modal::ChapterResults { cursor, .. } = &state.ui.modal {
            assert_eq!(*cursor, 0);
        }
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
                chapters: vec![hit("u1", "Book A", 2, 0.9)],
                best_score: 0.9,
            }],
            cursor: 0,
            scroll_offset: 0,
            status: SearchStatus::Ready,
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
                chapters: vec![hit("u-unknown", "Remote Book", 5, 0.9)],
                best_score: 0.9,
            }],
            cursor: 0,
            scroll_offset: 0,
            status: SearchStatus::Ready,
        };
        handle_chapter_results(&mut state, key_event(KEY_ENTER));
        assert_eq!(state.ui.modal, Modal::None);
        assert_eq!(state.ui.page, Page::Reader);
        assert_eq!(state.reader.session_id, -1);
        assert_eq!(state.reader.session_name, "Remote Book");
        assert!(!state.reader.loading);
    }

    #[test]
    fn test_enter_with_no_selection_closes() {
        let mut state = test_state();
        state.ui.modal = Modal::ChapterResults {
            query: "dragon".into(),
            groups: vec![],
            cursor: 0,
            scroll_offset: 0,
            status: SearchStatus::Empty,
        };
        handle_chapter_results(&mut state, key_event(KEY_ENTER));
        assert_eq!(state.ui.modal, Modal::None);
    }

    #[test]
    fn test_unhandled_key_returns_true() {
        let mut state = results_state();
        let result = handle_chapter_results(&mut state, key_event(KeyCode::Char('x')));
        assert!(result);
    }
}
