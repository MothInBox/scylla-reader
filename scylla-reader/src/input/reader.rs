//! Reader page input handler — paging, scrolling, chapter nav.

use crate::messenger::AppCommand;
use crate::state::AppState;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::prelude::Rect;

pub fn handle_reader(
    state: &mut AppState,
    key: KeyEvent,
    cmd_tx: &std::sync::mpsc::Sender<AppCommand>,
    size: Rect,
) -> bool {
    use crate::settings::ReaderMode;

    match (key.modifiers, key.code) {
        (_, KeyCode::Char('>')) => {
            if !state.reader.loading {
                if let Some(book) = state.library.selected_book() {
                    let next_idx = state.reader.current_chapter_idx + 1;
                    if let Some(ch) = book.chapters.get(next_idx) {
                        let url = ch.url.clone();
                        state.reader.loading = true;
                        let _ = cmd_tx.send(AppCommand::FetchChapter(url, next_idx));
                    }
                }
            }
            return true;
        }
        (_, KeyCode::Char('<')) => {
            if !state.reader.loading {
                if let Some(book) = state.library.selected_book() {
                    let prev_idx = state.reader.current_chapter_idx.saturating_sub(1);
                    if prev_idx != state.reader.current_chapter_idx {
                        if let Some(ch) = book.chapters.get(prev_idx) {
                            let url = ch.url.clone();
                            state.reader.loading = true;
                            let _ = cmd_tx.send(AppCommand::FetchChapter(url, prev_idx));
                        }
                    }
                }
            }
            return true;
        }
        _ => {}
    }

    match state.settings.reader_mode {
        ReaderMode::Paged => match key.code {
            KeyCode::Right => {
                state
                    .reader
                    .next_page(size.width, size.height.saturating_sub(2));
                true
            }
            KeyCode::Left => {
                state
                    .reader
                    .prev_page(size.width, size.height.saturating_sub(2));
                true
            }
            _ => true,
        },
        ReaderMode::Scrollable => match key.code {
            KeyCode::Down => {
                state.reader.scroll_down_visual(size.width);
                true
            }
            KeyCode::Up => {
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
    use crate::db::Db;
    use crate::library::Library;
    use crate::models::Chapter;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::prelude::Rect;

    fn test_state() -> AppState {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let db = Db::open_conn(conn).unwrap();
        AppState::from_parts(db, Library::new())
    }

    fn key_event(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn channel() -> (
        std::sync::mpsc::Sender<AppCommand>,
        std::sync::mpsc::Receiver<AppCommand>,
    ) {
        std::sync::mpsc::channel()
    }

    fn rect() -> Rect {
        Rect::new(0, 0, 80, 24)
    }

    fn state_with_book_and_chapters() -> AppState {
        let mut state = test_state();
        state.library.add_book("Test Book".into(), "url".into(), 10);
        state.library.books[0].chapters.extend(vec![
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
        let (tx, rx) = channel();
        let result = handle_reader(&mut state, key_event(KeyCode::Char('>')), &tx, rect());
        assert!(result);
        assert!(state.reader.loading);
        match rx.try_recv() {
            Ok(AppCommand::FetchChapter(url, idx)) => {
                assert_eq!(url, "url-2");
                assert_eq!(idx, 1);
            }
            _ => panic!("Expected FetchChapter"),
        }
    }

    #[test]
    fn test_handle_reader_lt_prev_chapter() {
        let mut state = state_with_book_and_chapters();
        state.reader.current_chapter_idx = 1;
        let (tx, rx) = channel();
        let result = handle_reader(&mut state, key_event(KeyCode::Char('<')), &tx, rect());
        assert!(result);
        assert!(state.reader.loading);
        match rx.try_recv() {
            Ok(AppCommand::FetchChapter(url, idx)) => {
                assert_eq!(url, "url-1");
                assert_eq!(idx, 0);
            }
            _ => panic!("Expected FetchChapter"),
        }
    }

    #[test]
    fn test_handle_reader_prev_chapter_clamps_at_zero() {
        let mut state = state_with_book_and_chapters();
        let (tx, rx) = channel();
        let result = handle_reader(&mut state, key_event(KeyCode::Char('<')), &tx, rect());
        assert!(result);
        assert!(!state.reader.loading);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn test_handle_reader_next_chapter_clamps_at_last() {
        let mut state = state_with_book_and_chapters();
        state.reader.current_chapter_idx = 2;
        let (tx, rx) = channel();
        let result = handle_reader(&mut state, key_event(KeyCode::Char('>')), &tx, rect());
        assert!(result);
        assert!(!state.reader.loading);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn test_handle_reader_loading_guard_prevents_chapter_nav() {
        let mut state = state_with_book_and_chapters();
        state.reader.loading = true;
        let (tx, rx) = channel();
        let result = handle_reader(&mut state, key_event(KeyCode::Char('>')), &tx, rect());
        assert!(result);
        assert!(state.reader.loading);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn test_handle_reader_paged_mode_unhandled_key() {
        let mut state = test_state();
        state.settings.reader_mode = crate::settings::ReaderMode::Paged;
        let (tx, _rx) = channel();
        let result = handle_reader(&mut state, key_event(KeyCode::Char('z')), &tx, rect());
        assert!(result);
    }

    #[test]
    fn test_handle_reader_scrollable_down() {
        let mut state = test_state();
        state.settings.reader_mode = crate::settings::ReaderMode::Scrollable;
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
        let (tx, _rx) = channel();
        let result = handle_reader(&mut state, key_event(KeyCode::Down), &tx, rect());
        assert!(result);
        assert_eq!(state.reader.visual_scroll, 1);
    }

    #[test]
    fn test_handle_reader_scrollable_up() {
        let mut state = test_state();
        state.settings.reader_mode = crate::settings::ReaderMode::Scrollable;
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
        let (tx, _rx) = channel();
        let result = handle_reader(&mut state, key_event(KeyCode::Up), &tx, rect());
        assert!(result);
        assert_eq!(state.reader.visual_scroll, 9);
    }
}
