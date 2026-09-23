//! Library page input handler — navigation, filter, add/delete, jump.

use crate::event_types::{ChapterGroup, ChapterHit};
use crate::input::keybinds::*;
use crate::library::AiRow;
use crate::state::modal::{FilterRow, SearchStatus};
use crate::state::{LibraryState, Modal, UiState};
use crossterm::event::KeyEvent;

pub fn handle_library(ui: &mut UiState, lib: &mut LibraryState, key: KeyEvent) -> bool {
    match key.code {
        KEY_ADD_BOOK => {
            ui.modal = Modal::AddBook {
                inputs: vec![String::new()],
                cursor: 0,
                scroll_offset: 0,
            };
            true
        }
        KEY_JUMP_CHAPTER => {
            if let Some(book) = lib.library.selected_book() {
                crate::settings::log(
                    crate::settings::LogLevel::Debug,
                    "INPUT",
                    &format!("Adding Book {} to modal", book.title),
                );
                ui.modal = Modal::JumpChapter {
                    chapters: book.chapters.clone(),
                    query: String::new(),
                    filtered: book.chapters.clone(),
                    cursor: 0,
                    scroll_offset: 0,
                    show_titles: true,
                };
            }
            true
        }
        KEY_EMBED => {
            open_embed_chapters(ui, lib);
            true
        }
        KEY_DELETE => {
            lib.library.remove_selected();
            true
        }
        KEY_CYCLE_STATUS => {
            lib.library.cycle_selected_status();
            true
        }
        KEY_FILTER => {
            // `f` while AI-ranked opens the filter modal seeded with the AI
            // query and focused on the AI row (refine). Otherwise the normal
            // filter modal.
            let ai_active = lib.library.ai.is_some();
            let ai_query = lib
                .library
                .ai
                .as_ref()
                .map(|ai| ai.query.clone())
                .unwrap_or_default();
            ui.modal = Modal::Filter {
                working: lib.library.filter.clone(),
                focus: if ai_active {
                    FilterRow::Ai
                } else {
                    FilterRow::Search
                },
                tag_query: String::new(),
                tag_cursor: 0,
                tag_scroll: 0,
                status_cursor: lib.library.filter.status_cursor(),
                lib_cursor: lib
                    .library
                    .filter
                    .library_cursor(&lib.manager.backend_names()),
                ai_query,
            };
            true
        }
        KEY_SESSIONS => {
            if lib.library.ai.is_some() {
                open_ai_drilldown(ui, lib);
            } else if let Some(book) = lib.library.selected_book() {
                ui.modal = Modal::SessionPicker {
                    book_url: book.url.clone(),
                    cursor: 0,
                    scroll_offset: 0,
                    input: None,
                    editing_id: None,
                    pending_delete_url: None,
                };
            }
            true
        }
        KEY_ESCAPE => {
            // Esc with an active AI session clears it and restores normal
            // ranking. Without AI, Esc is a no-op (unchanged).
            if lib.library.ai.is_some() {
                lib.library.clear_ai();
            }
            true
        }
        KEY_AI_TOGGLE => {
            // `g` toggles book-mode ↔ grouped-chapter mode (AI only).
            if lib.library.ai.is_some() {
                lib.library.toggle_ai_mode();
            }
            true
        }
        KEY_UPDATE_ALL => {
            let urls: Vec<String> = lib
                .library
                .books
                .iter()
                .map(|b| b.url.clone())
                .filter(|u| !u.is_empty())
                .collect();
            if !urls.is_empty() {
                crate::settings::log(
                    crate::settings::LogLevel::Debug,
                    "INPUT",
                    "Attempting to update all books...",
                );
                let base = crate::storage::client::api_base_for(&lib.manager);
                for url in urls {
                    if let Err(e) = crate::storage::client::block_on(
                        crate::storage::client::enqueue_job(&base, "Scrape", &url, None),
                    ) {
                        crate::settings::log(
                            crate::settings::LogLevel::Error,
                            "INPUT",
                            &format!("Failed to enqueue scrape: {}", e),
                        );
                    }
                }
            }
            true
        }
        KEY_NAV_DOWN => {
            if lib.library.ai.is_some() {
                lib.library.ai_nav(1);
            } else {
                let visible_len = lib.library.visible_indices().len();
                if visible_len > 0 {
                    lib.library.selected_index =
                        (lib.library.selected_index + 1).min(visible_len - 1);
                }
            }
            true
        }
        KEY_NAV_UP => {
            if lib.library.ai.is_some() {
                lib.library.ai_nav(-1);
            } else if !lib.library.visible_indices().is_empty() {
                lib.library.selected_index = lib.library.selected_index.saturating_sub(1);
            }
            true
        }
        _ => true,
    }
}

/// Enter on an AI row: drill into the selected book's inline chapters via the
/// chapter-results modal (a local filter of already-received data — no network,
/// no spinner). The modal's Enter opens a chapter.
fn open_ai_drilldown(ui: &mut UiState, lib: &mut LibraryState) {
    let Some(ai) = &lib.library.ai else {
        return;
    };
    let rows = ai.rows();
    let Some(row) = rows.get(ai.cursor) else {
        return;
    };
    let (book_index, chapter_index) = match row {
        AiRow::Book { book_index } => (*book_index, 0),
        AiRow::Chapter {
            book_index,
            chapter_index,
        } => (*book_index, *chapter_index),
    };
    let Some(book) = ai.results.books.get(book_index) else {
        return;
    };
    let group = ai_book_to_group(book);
    let query = ai.query.clone();
    // Show the book's chapters expanded; the cursor lands on the selected
    // chapter (or the first), so one more Enter opens it. A book with no
    // matching chapters gets the "no matching chapters" hint, not an empty list.
    let status = if group.chapters.is_empty() {
        SearchStatus::NoChapters
    } else {
        SearchStatus::Ready
    };
    let cursor = chapter_index.min(group.chapters.len().saturating_sub(1));
    ui.modal = Modal::ChapterResults {
        query,
        groups: vec![group],
        cursor,
        scroll_offset: 0,
        status,
        expanded: Some(0),
    };
}

/// Convert an AI book (with inline chapters) into the modal's chapter group.
fn ai_book_to_group(book: &crate::event_types::AiBook) -> ChapterGroup {
    let chapters: Vec<ChapterHit> = book
        .chapters
        .iter()
        .map(|c| ChapterHit {
            book_url: book.book_url.clone(),
            book_title: book.book_title.clone(),
            chapter_url: c.chapter_url.clone(),
            chapter_idx: c.chapter_idx,
            chapter_title: c.chapter_title.clone(),
            score: c.score,
            genres: book.genres.clone(),
        })
        .collect();
    ChapterGroup {
        book_url: book.book_url.clone(),
        book_title: book.book_title.clone(),
        genres: book.genres.clone(),
        chapters,
        best_score: book.score,
    }
}

/// Open the embed-chapters modal for the selected book. Shared by the `e` key
/// handler and the command palette "Embed Chapters…" action.
pub(crate) fn open_embed_chapters(ui: &mut UiState, lib: &mut LibraryState) {
    let book = lib
        .library
        .selected_book()
        .map(|b| (b.url.clone(), b.title.clone(), b.chapters.clone()));
    let Some((book_url, book_title, chapters)) = book else {
        return;
    };
    crate::settings::log(
        crate::settings::LogLevel::Debug,
        "INPUT",
        &format!("Opening embed-chapters for {}", book_title),
    );
    // Always fetch a fresh embedding status so the modal reflects chapters
    // embedded since the last fetch; on error fall back to the cached status
    // (or empty).
    let base = crate::storage::client::api_base_for(&lib.manager);
    let fetch = crate::storage::client::block_on(crate::storage::client::embedding_status(
        &base, &book_url,
    ));
    let embedded_urls =
        resolve_embedded_urls(&mut lib.library.embedding_status_cache, &book_url, fetch);
    // Start the cursor on the first selectable (non-embedded) row.
    let initial_cursor = crate::input::modal::first_selectable(&chapters, &embedded_urls);
    let selected = vec![false; chapters.len()];
    ui.modal = Modal::EmbedChapters {
        book_url,
        chapters,
        selected,
        embedded_urls,
        cursor: initial_cursor,
        scroll_offset: 0,
    };
}

/// Resolve the embedded chapter URLs for the embed modal from a fresh fetch:
/// on success, update the cache and return the fresh URLs; on error, fall back
/// to the cached status if present, else empty.
fn resolve_embedded_urls(
    cache: &mut std::collections::HashMap<String, crate::storage::client::EmbeddingStatus>,
    book_url: &str,
    fetch: Result<crate::storage::client::EmbeddingStatus, String>,
) -> Vec<String> {
    match fetch {
        Ok(status) => {
            cache.insert(book_url.to_string(), status.clone());
            status.embedded_chapter_urls
        }
        Err(e) => {
            crate::settings::log(
                crate::settings::LogLevel::Debug,
                "INPUT",
                &format!("Failed to fetch embedding status for {}: {}", book_url, e),
            );
            cache
                .get(book_url)
                .map(|s| s.embedded_chapter_urls.clone())
                .unwrap_or_default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Chapter;
    use crate::models::book::BookStatus;
    use crate::state::AppState;
    use crate::state::Page;
    use crate::test_helpers::*;
    use crossterm::event::KeyCode;

    #[test]
    fn test_handle_library_i_opens_add_book() {
        let mut state = test_state();
        let result = handle_library(&mut state.ui, &mut state.lib, key_event(KEY_ADD_BOOK));
        assert!(result);
        assert!(matches!(state.ui.modal, Modal::AddBook { .. }));
        assert_eq!(
            state.ui.modal,
            Modal::AddBook {
                inputs: vec![String::new()],
                cursor: 0,
                scroll_offset: 0,
            }
        );
    }

    #[test]
    fn test_handle_library_j_opens_jump_chapter() {
        let mut state = test_state();
        state.lib.library.add_book("Test".into(), "url".into());
        state.lib.library.books[0].chapters.push(Chapter {
            title: "Ch1".into(),
            url: "u1".into(),
            order: 0,
        });
        let result = handle_library(&mut state.ui, &mut state.lib, key_event(KEY_JUMP_CHAPTER));
        assert!(result);
        assert!(matches!(state.ui.modal, Modal::JumpChapter { .. }));
        assert_eq!(
            state.ui.modal,
            Modal::JumpChapter {
                chapters: vec![Chapter {
                    title: "Ch1".into(),
                    url: "u1".into(),
                    order: 0
                }],
                query: String::new(),
                filtered: vec![Chapter {
                    title: "Ch1".into(),
                    url: "u1".into(),
                    order: 0
                }],
                cursor: 0,
                scroll_offset: 0,
                show_titles: true,
            }
        );
    }

    #[test]
    fn test_handle_library_u_updates_all() {
        let mut state = test_state();
        state.lib.library.add_book("A".into(), "url-a".into());
        state.lib.library.add_book("B".into(), "url-b".into());
        let result = handle_library(&mut state.ui, &mut state.lib, key_event(KEY_UPDATE_ALL));
        assert!(result);
    }

    #[test]
    fn test_handle_library_d_deletes() {
        let mut state = test_state();
        state.lib.library.add_book("Test".into(), "url".into());
        assert_eq!(state.lib.library.books.len(), 1);
        let result = handle_library(&mut state.ui, &mut state.lib, key_event(KEY_DELETE));
        assert!(result);
        assert_eq!(state.lib.library.books.len(), 0);
    }

    #[test]
    fn test_handle_library_f_opens_filter_modal() {
        let mut state = test_state();
        state.lib.library.filter.name = "glo".into();
        let result = handle_library(&mut state.ui, &mut state.lib, key_event(KEY_FILTER));
        assert!(result);
        if let Modal::Filter {
            working,
            focus,
            tag_query,
            tag_cursor,
            tag_scroll,
            status_cursor,
            lib_cursor,
            ai_query,
        } = &state.ui.modal
        {
            assert_eq!(working.name, "glo");
            assert_eq!(*focus, crate::state::modal::FilterRow::Search);
            assert!(tag_query.is_empty());
            assert_eq!(*tag_cursor, 0);
            assert_eq!(*tag_scroll, 0);
            assert_eq!(*status_cursor, 0);
            assert_eq!(*lib_cursor, 0);
            assert!(ai_query.is_empty());
        } else {
            panic!("Expected Filter modal");
        }
    }

    #[test]
    fn test_handle_library_f_seeds_cursors_from_filter() {
        let mut state = test_state_with_backend(Box::new(MockBackend::new("remote1")));
        state.lib.library.filter.status = Some(BookStatus::Dropped);
        state.lib.library.filter.library = Some("remote1".into());
        let result = handle_library(&mut state.ui, &mut state.lib, key_event(KEY_FILTER));
        assert!(result);
        if let Modal::Filter {
            status_cursor,
            lib_cursor,
            ..
        } = &state.ui.modal
        {
            assert_eq!(*status_cursor, 3);
            assert_eq!(*lib_cursor, 1);
        } else {
            panic!("Expected Filter modal");
        }
    }

    #[test]
    fn test_handle_library_space_cycles_status() {
        let mut state = test_state();
        state.lib.library.add_book("Test".into(), "url".into());
        assert_eq!(state.lib.library.books[0].status, BookStatus::Reading);
        let result = handle_library(&mut state.ui, &mut state.lib, key_event(KEY_CYCLE_STATUS));
        assert!(result);
        assert_eq!(state.lib.library.books[0].status, BookStatus::Paused);
    }

    #[test]
    fn test_handle_library_enter_opens_session_picker() {
        let mut state = test_state();
        state.lib.library.add_book("Test".into(), "url".into());
        let result = handle_library(&mut state.ui, &mut state.lib, key_event(KEY_SESSIONS));
        assert!(result);
        assert_eq!(state.ui.page, Page::Library);
        assert!(matches!(state.ui.modal, Modal::SessionPicker { .. }));
    }

    #[test]
    fn test_handle_library_up_down_navigation() {
        let mut state = test_state();
        state.lib.library.add_book("A".into(), "u1".into());
        state.lib.library.add_book("B".into(), "u2".into());
        state.lib.library.add_book("C".into(), "u3".into());
        assert_eq!(state.lib.library.selected_index, 0);
        handle_library(&mut state.ui, &mut state.lib, key_event(KEY_NAV_DOWN));
        assert_eq!(state.lib.library.selected_index, 1);
        handle_library(&mut state.ui, &mut state.lib, key_event(KEY_NAV_DOWN));
        assert_eq!(state.lib.library.selected_index, 2);
        handle_library(&mut state.ui, &mut state.lib, key_event(KEY_NAV_UP));
        assert_eq!(state.lib.library.selected_index, 1);
        handle_library(&mut state.ui, &mut state.lib, key_event(KEY_NAV_UP));
        assert_eq!(state.lib.library.selected_index, 0);
    }

    #[test]
    fn test_handle_library_up_does_not_go_below_zero() {
        let mut state = test_state();
        state.lib.library.add_book("A".into(), "u1".into());
        assert_eq!(state.lib.library.selected_index, 0);
        handle_library(&mut state.ui, &mut state.lib, key_event(KEY_NAV_UP));
        assert_eq!(state.lib.library.selected_index, 0);
    }

    #[test]
    fn test_handle_library_down_does_not_exceed_max() {
        let mut state = test_state();
        state.lib.library.add_book("A".into(), "u1".into());
        state.lib.library.add_book("B".into(), "u2".into());
        handle_library(&mut state.ui, &mut state.lib, key_event(KEY_NAV_DOWN));
        handle_library(&mut state.ui, &mut state.lib, key_event(KEY_NAV_DOWN));
        assert_eq!(state.lib.library.selected_index, 1);
    }

    #[test]
    fn test_handle_library_unhandled_key_returns_true() {
        let mut state = test_state();
        let result = handle_library(&mut state.ui, &mut state.lib, key_event(KeyCode::Char('x')));
        assert!(result);
    }

    #[test]
    fn test_handle_library_e_opens_embed_chapters_on_first_selectable() {
        // MockBackend → api_base is "http://mock" (non-resolvable), so the
        // fresh fetch always fails and the opener falls back to the cache.
        let mut state = test_state_with_backend(Box::new(MockBackend::new("mock")));
        state.lib.library.add_book("Test".into(), "url".into());
        if let Some(book) = state.lib.library.books.iter_mut().find(|b| b.url == "url") {
            book.chapters = vec![
                crate::models::Chapter {
                    title: "Ch0".into(),
                    url: "u0".into(),
                    order: 0,
                },
                crate::models::Chapter {
                    title: "Ch1".into(),
                    url: "u1".into(),
                    order: 1,
                },
                crate::models::Chapter {
                    title: "Ch2".into(),
                    url: "u2".into(),
                    order: 2,
                },
            ];
        }
        // ch0 and ch1 embedded → the cursor must land on ch2 (first selectable).
        state.lib.library.embedding_status_cache.insert(
            "url".into(),
            crate::storage::client::EmbeddingStatus {
                embedded_chapters: 2,
                total_chapters: 3,
                aggregate: true,
                genres: vec![],
                embedded_chapter_urls: vec!["u0".into(), "u1".into()],
            },
        );
        let result = handle_library(&mut state.ui, &mut state.lib, key_event(KEY_EMBED));
        assert!(result);
        match &state.ui.modal {
            Modal::EmbedChapters {
                cursor,
                embedded_urls,
                ..
            } => {
                assert_eq!(*cursor, 2);
                assert_eq!(embedded_urls, &vec!["u0".to_string(), "u1".to_string()]);
            }
            _ => panic!("Expected EmbedChapters modal"),
        }
    }

    #[test]
    fn test_resolve_embedded_urls_fresh_updates_cache() {
        let mut cache = std::collections::HashMap::new();
        let fresh = crate::storage::client::EmbeddingStatus {
            embedded_chapters: 3,
            total_chapters: 3,
            aggregate: true,
            genres: vec![],
            embedded_chapter_urls: vec!["u0".into(), "u1".into(), "u2".into()],
        };
        let urls = resolve_embedded_urls(&mut cache, "url", Ok(fresh.clone()));
        assert_eq!(urls, vec!["u0", "u1", "u2"]);
        // The cache is updated with the fresh status.
        assert_eq!(cache.get("url"), Some(&fresh));
    }

    #[test]
    fn test_resolve_embedded_urls_error_falls_back_to_cache() {
        let mut cache = std::collections::HashMap::new();
        cache.insert(
            "url".into(),
            crate::storage::client::EmbeddingStatus {
                embedded_chapters: 1,
                total_chapters: 3,
                aggregate: false,
                genres: vec![],
                embedded_chapter_urls: vec!["u0".into()],
            },
        );
        let urls = resolve_embedded_urls(&mut cache, "url", Err("boom".into()));
        assert_eq!(urls, vec!["u0"]);
    }

    #[test]
    fn test_resolve_embedded_urls_error_without_cache_is_empty() {
        let mut cache = std::collections::HashMap::new();
        let urls = resolve_embedded_urls(&mut cache, "url", Err("boom".into()));
        assert!(urls.is_empty());
    }

    // ── AI session input ────────────────────────────────────────────────────

    /// A state with two library books and an active book-mode AI session.
    fn ai_state() -> AppState {
        let mut state = test_state();
        state.lib.library.add_book("Book A".into(), "u1".into());
        state.lib.library.add_book("Book B".into(), "u2".into());
        state.lib.library.apply_ai_results(
            "dragon heart".into(),
            crate::event_types::SearchOutcome::BookMode {
                embedded: 2,
                total: 2,
                hits: vec![
                    crate::event_types::AiBook {
                        book_url: "u2".into(),
                        book_title: "Book B".into(),
                        score: 90.0,
                        genres: vec![],
                        chapters: vec![crate::event_types::AiChapter {
                            chapter_url: "u2/ch0".into(),
                            chapter_idx: 0,
                            chapter_title: "Scene 1".into(),
                            score: 90.0,
                        }],
                    },
                    crate::event_types::AiBook {
                        book_url: "u1".into(),
                        book_title: "Book A".into(),
                        score: 70.0,
                        genres: vec![],
                        chapters: vec![],
                    },
                ],
            },
        );
        state
    }

    #[test]
    fn test_enter_with_ai_opens_drilldown_modal() {
        let mut state = ai_state();
        // Cursor on Book B (index 0) → drill into its chapters.
        let result = handle_library(&mut state.ui, &mut state.lib, key_event(KEY_SESSIONS));
        assert!(result);
        match &state.ui.modal {
            Modal::ChapterResults {
                query,
                groups,
                expanded,
                status,
                ..
            } => {
                assert_eq!(query, "dragon heart");
                assert_eq!(*expanded, Some(0));
                assert_eq!(*status, SearchStatus::Ready);
                assert_eq!(groups.len(), 1);
                assert_eq!(groups[0].book_title, "Book B");
                assert_eq!(groups[0].chapters[0].chapter_title, "Scene 1");
            }
            _ => panic!("expected ChapterResults drill-down"),
        }
        // The AI session is preserved (drill-down is a local view).
        assert!(state.lib.library.ai.is_some());
    }

    #[test]
    fn test_enter_with_ai_on_chapter_lands_cursor_on_chapter() {
        let mut state = ai_state();
        state.lib.library.toggle_ai_mode(); // grouped-chapter
        // Cursor: Book B header (0), its chapter (1).
        state.lib.library.ai_nav(1);
        let result = handle_library(&mut state.ui, &mut state.lib, key_event(KEY_SESSIONS));
        assert!(result);
        if let Modal::ChapterResults {
            cursor, expanded, ..
        } = &state.ui.modal
        {
            assert_eq!(*cursor, 0);
            assert_eq!(*expanded, Some(0));
        } else {
            panic!("expected ChapterResults drill-down");
        }
    }

    #[test]
    fn test_esc_with_ai_clears_ai() {
        let mut state = ai_state();
        let result = handle_library(&mut state.ui, &mut state.lib, key_event(KEY_ESCAPE));
        assert!(result);
        assert!(state.lib.library.ai.is_none());
        assert_eq!(state.lib.library.visible_indices(), vec![0, 1]);
    }

    #[test]
    fn test_esc_without_ai_is_noop() {
        let mut state = test_state();
        state.lib.library.add_book("A".into(), "u".into());
        let result = handle_library(&mut state.ui, &mut state.lib, key_event(KEY_ESCAPE));
        assert!(result);
        assert!(state.lib.library.ai.is_none());
        assert_eq!(state.ui.page, crate::state::Page::Library);
    }

    #[test]
    fn test_g_toggles_ai_mode() {
        let mut state = ai_state();
        assert!(state.lib.library.ai.as_ref().unwrap().book_mode);
        handle_library(&mut state.ui, &mut state.lib, key_event(KEY_AI_TOGGLE));
        assert!(!state.lib.library.ai.as_ref().unwrap().book_mode);
        handle_library(&mut state.ui, &mut state.lib, key_event(KEY_AI_TOGGLE));
        assert!(state.lib.library.ai.as_ref().unwrap().book_mode);
    }

    #[test]
    fn test_g_without_ai_is_noop() {
        let mut state = test_state();
        let result = handle_library(&mut state.ui, &mut state.lib, key_event(KEY_AI_TOGGLE));
        assert!(result);
        assert!(state.lib.library.ai.is_none());
    }

    #[test]
    fn test_f_with_ai_seeds_ai_query_and_focus() {
        let mut state = ai_state();
        handle_library(&mut state.ui, &mut state.lib, key_event(KEY_FILTER));
        match &state.ui.modal {
            Modal::Filter {
                focus, ai_query, ..
            } => {
                assert_eq!(*focus, FilterRow::Ai);
                assert_eq!(ai_query, "dragon heart");
            }
            _ => panic!("expected Filter modal"),
        }
    }

    #[test]
    fn test_ai_navigation_moves_cursor() {
        let mut state = ai_state();
        assert_eq!(state.lib.library.ai.as_ref().unwrap().cursor, 0);
        handle_library(&mut state.ui, &mut state.lib, key_event(KEY_NAV_DOWN));
        assert_eq!(state.lib.library.ai.as_ref().unwrap().cursor, 1);
        handle_library(&mut state.ui, &mut state.lib, key_event(KEY_NAV_UP));
        assert_eq!(state.lib.library.ai.as_ref().unwrap().cursor, 0);
    }
}
