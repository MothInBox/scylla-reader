//! Library page input handler — navigation, filter, add/delete, jump.

use crate::input::keybinds::*;
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
        KEY_BACKENDS => {
            ui.modal = Modal::BackendPicker {
                cursor: 0,
                scroll_offset: 0,
                input: None,
                editing_idx: None,
                pending_delete_idx: None,
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
            if let Some(book) = lib.library.selected_book() {
                crate::settings::log(
                    crate::settings::LogLevel::Debug,
                    "INPUT",
                    &format!("Opening embed-chapters for {}", book.title),
                );
                // Already-embedded chapter URLs come from the cached status, or
                // a synchronous fetch when not cached yet.
                let embedded_urls =
                    if let Some(status) = lib.library.embedding_status_cache.get(&book.url) {
                        status.embedded_chapter_urls.clone()
                    } else {
                        let base = crate::storage::client::api_base_for(&lib.manager);
                        match crate::storage::client::block_on(
                            crate::storage::client::embedding_status(&base, &book.url),
                        ) {
                            Ok(status) => status.embedded_chapter_urls,
                            Err(e) => {
                                crate::settings::log(
                                    crate::settings::LogLevel::Debug,
                                    "INPUT",
                                    &format!(
                                        "Failed to fetch embedding status for {}: {}",
                                        book.url, e
                                    ),
                                );
                                Vec::new()
                            }
                        }
                    };
                // Start the cursor on the first selectable (non-embedded) row.
                let initial_cursor = book
                    .chapters
                    .iter()
                    .position(|ch| !embedded_urls.contains(&ch.url))
                    .unwrap_or(0);
                ui.modal = Modal::EmbedChapters {
                    book_url: book.url.clone(),
                    chapters: book.chapters.clone(),
                    selected: vec![false; book.chapters.len()],
                    embedded_urls,
                    cursor: initial_cursor,
                    scroll_offset: 0,
                };
            }
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
            ui.modal = Modal::Filter {
                working: lib.library.filter.clone(),
                focus: crate::state::modal::FilterRow::Search,
                tag_query: String::new(),
                tag_cursor: 0,
                tag_scroll: 0,
                status_cursor: lib.library.filter.status_cursor(),
                lib_cursor: lib
                    .library
                    .filter
                    .library_cursor(&lib.manager.backend_names()),
                ai_query: String::new(),
            };
            true
        }
        KEY_SESSIONS => {
            if let Some(book) = lib.library.selected_book() {
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
            let visible_len = lib.library.visible_indices().len();
            if visible_len > 0 {
                lib.library.selected_index = (lib.library.selected_index + 1).min(visible_len - 1);
            }
            true
        }
        KEY_NAV_UP => {
            if !lib.library.visible_indices().is_empty() {
                lib.library.selected_index = lib.library.selected_index.saturating_sub(1);
            }
            true
        }
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Chapter;
    use crate::models::book::BookStatus;
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
}
