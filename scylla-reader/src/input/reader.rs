//! Reader page input handler — paging, scrolling, chapter nav.

use crate::input::keybinds::*;
use crate::state::AppState;
use crate::state::Modal;
use crossterm::event::KeyEvent;
use ratatui::prelude::Rect;

pub fn handle_reader(state: &mut AppState, key: KeyEvent, size: Rect) -> bool {
    use crate::settings::ReaderMode;

    match (key.modifiers, key.code) {
        (_, KEY_NEXT_CHAPTER) => {
            if !state.reader.loading {
                let next_idx = state.reader.current_chapter_idx.saturating_add(1);
                let chapter = state
                    .lib
                    .library
                    .selected_book()
                    .and_then(|b| b.chapters.get(next_idx).cloned());
                if let Some(ch) = chapter {
                    let url = ch.url.clone();
                    state.reader.loading = true;
                    let base = crate::storage::client::api_base(state);
                    if let Err(e) =
                        crate::storage::client::block_on(crate::storage::client::enqueue_job(
                            &base,
                            "FetchChapter",
                            &url,
                            Some(next_idx),
                        ))
                    {
                        crate::settings::log(
                            crate::settings::LogLevel::Error,
                            "INPUT",
                            &format!("Failed to enqueue chapter fetch: {}", e),
                        );
                        state.reader.loading = false;
                    }
                }
            }
            return true;
        }
        (_, KEY_PREV_CHAPTER) => {
            if !state.reader.loading {
                let prev_idx = state.reader.current_chapter_idx.saturating_sub(1);
                let chapter = if prev_idx != state.reader.current_chapter_idx {
                    state
                        .lib
                        .library
                        .selected_book()
                        .and_then(|b| b.chapters.get(prev_idx).cloned())
                } else {
                    None
                };
                if let Some(ch) = chapter {
                    let url = ch.url.clone();
                    state.reader.loading = true;
                    let base = crate::storage::client::api_base(state);
                    if let Err(e) =
                        crate::storage::client::block_on(crate::storage::client::enqueue_job(
                            &base,
                            "FetchChapter",
                            &url,
                            Some(prev_idx),
                        ))
                    {
                        crate::settings::log(
                            crate::settings::LogLevel::Error,
                            "INPUT",
                            &format!("Failed to enqueue chapter fetch: {}", e),
                        );
                        state.reader.loading = false;
                    }
                }
            }
            return true;
        }
        (_, KEY_MANAGE_SESSIONS) => {
            if let Some(book) = state.lib.library.selected_book() {
                state.ui.modal = Modal::SessionPicker {
                    book_url: book.url.clone(),
                    cursor: 0,
                    scroll_offset: 0,
                    input: None,
                    editing_id: None,
                    pending_delete_url: None,
                };
            }
            return true;
        }
        _ => {}
    }

    match state.lib.settings.reader_mode {
        ReaderMode::Paged => match key.code {
            KEY_NEXT_PAGE => {
                state.reader.next_page(size.width, size.height);
                true
            }
            KEY_PREV_PAGE => {
                state.reader.prev_page(size.height);
                true
            }
            _ => true,
        },
        ReaderMode::Scrollable => match key.code {
            KEY_SCROLL_DOWN => {
                state.reader.scroll_down_visual(size.width);
                true
            }
            KEY_SCROLL_UP => {
                state.reader.scroll_up_visual();
                true
            }
            _ => true,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Chapter;
    use crate::test_helpers::*;
    use crossterm::event::KeyCode;

    fn state_with_book_and_chapters() -> AppState {
        // MockBackend's url() is "http://mock" (non-resolvable) so the
        // FetchChapter enqueue fails deterministically regardless of whether a
        // real server is running on 127.0.0.1:8080.
        let mut state = test_state_with_backend(Box::new(MockBackend::new("mock")));
        state.lib.library.add_book("Test Book".into(), "url".into());
        state.lib.library.books[0].chapters.extend(vec![
            Chapter {
                title: "Ch1".into(),
                url: "url-1".into(),
                order: 0,
            },
            Chapter {
                title: "Ch2".into(),
                url: "url-2".into(),
                order: 1,
            },
            Chapter {
                title: "Ch3".into(),
                url: "url-3".into(),
                order: 2,
            },
        ]);
        state
    }

    #[test]
    fn test_handle_reader_gt_next_chapter() {
        let mut state = state_with_book_and_chapters();
        let result = handle_reader(&mut state, key_event(KEY_NEXT_CHAPTER), rect());
        assert!(result);
        // The enqueue fails in the test environment (no server), so the
        // loading flag must be cleared rather than left spinning.
        assert!(!state.reader.loading);
    }

    #[test]
    fn test_handle_reader_lt_prev_chapter() {
        let mut state = state_with_book_and_chapters();
        state.reader.current_chapter_idx = 1;
        let result = handle_reader(&mut state, key_event(KEY_PREV_CHAPTER), rect());
        assert!(result);
        // Enqueue fails in the test environment → loading cleared.
        assert!(!state.reader.loading);
    }

    #[test]
    fn test_handle_reader_prev_chapter_clamps_at_zero() {
        let mut state = state_with_book_and_chapters();
        let result = handle_reader(&mut state, key_event(KEY_PREV_CHAPTER), rect());
        assert!(result);
        assert!(!state.reader.loading);
    }

    #[test]
    fn test_handle_reader_next_chapter_clamps_at_last() {
        let mut state = state_with_book_and_chapters();
        state.reader.current_chapter_idx = 2;
        let result = handle_reader(&mut state, key_event(KEY_NEXT_CHAPTER), rect());
        assert!(result);
        assert!(!state.reader.loading);
    }

    #[test]
    fn test_handle_reader_loading_guard_prevents_chapter_nav() {
        let mut state = state_with_book_and_chapters();
        state.reader.loading = true;
        let result = handle_reader(&mut state, key_event(KEY_NEXT_CHAPTER), rect());
        assert!(result);
        assert!(state.reader.loading);
    }

    #[test]
    fn test_handle_reader_paged_mode_unhandled_key() {
        let mut state = test_state();
        state.lib.settings.reader_mode = crate::settings::ReaderMode::Paged;
        let result = handle_reader(&mut state, key_event(KeyCode::Char('z')), rect());
        assert!(result);
    }

    #[test]
    fn test_handle_reader_scrollable_down() {
        let mut state = test_state();
        state.lib.settings.reader_mode = crate::settings::ReaderMode::Scrollable;
        state.reader.load(
            "Book".into(),
            "url".into(),
            "Ch1".into(),
            (0..50)
                .map(|i| format!("line {}", i))
                .collect::<Vec<_>>()
                .join("\n"),
            0,
        );
        let result = handle_reader(&mut state, key_event(KEY_SCROLL_DOWN), rect());
        assert!(result);
        assert_eq!(state.reader.visual_scroll, 1);
    }

    #[test]
    fn test_handle_reader_scrollable_up() {
        let mut state = test_state();
        state.lib.settings.reader_mode = crate::settings::ReaderMode::Scrollable;
        state.reader.load(
            "Book".into(),
            "url".into(),
            "Ch1".into(),
            (0..50)
                .map(|i| format!("line {}", i))
                .collect::<Vec<_>>()
                .join("\n"),
            0,
        );
        state.reader.visual_scroll = 10;
        let result = handle_reader(&mut state, key_event(KEY_SCROLL_UP), rect());
        assert!(result);
        assert_eq!(state.reader.visual_scroll, 9);
    }
}
