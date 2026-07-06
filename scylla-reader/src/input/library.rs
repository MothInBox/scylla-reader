//! Library page input handler — navigation, filter, add/delete, jump.

use crate::messenger::AppCommand;
use crate::state::{AppState, Modal, Page};
use crossterm::event::{KeyCode, KeyEvent};

pub fn handle_library(
    state: &mut AppState,
    key: KeyEvent,
    cmd_tx: &std::sync::mpsc::Sender<AppCommand>,
) -> bool {
    match key.code {
        KeyCode::Char('i') => {
            state.modal = Modal::AddBook {
                inputs: vec![String::new()],
                cursor: 0,
                scroll_offset: 0,
            };
            state.current_page = Page::AddingBook;
            true
        }
        KeyCode::Char('j') => {
            if let Some(book) = state.library.selected_book() {
                crate::settings::log(
                    crate::settings::LogLevel::Debug,
                    "INPUT",
                    &format!("Adding Book {} to modal", book.title),
                );
                state.modal = Modal::JumpChapter {
                    chapters: book.chapters.clone(),
                    query: String::new(),
                    filtered: book.chapters.clone(),
                    cursor: 0,
                    scroll_offset: 0,
                    show_titles: true,
                };
                state.current_page = Page::BookChapterJump;
            }
            true
        }
        KeyCode::Char('d') => {
            state.library.remove_selected();
            true
        }
        KeyCode::Char(' ') => {
            state.library.cycle_selected_status();
            true
        }
        KeyCode::Char('f') => {
            state.library.cycle_filter();
            true
        }
        KeyCode::Enter => {
            if let Some(book) = state.library.selected_book() {
                state.modal = Modal::SessionPicker {
                    book_url: book.url.clone(),
                    cursor: 0,
                    scroll_offset: 0,
                    input: None,
                    editing_id: None,
                };
            }
            true
        }
        KeyCode::Char('u') => {
            let urls: Vec<String> = state
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
                if let Err(e) = cmd_tx.send(AppCommand::UpdateAll(urls)) {
                    crate::settings::log(
                        crate::settings::LogLevel::Error,
                        "INPUT",
                        &format!("Failed to queue update: {}", e),
                    );
                }
            }
            true
        }
        KeyCode::Down => {
            let visible_len = state.library.visible_indices().len();
            if visible_len > 0 {
                state.library.selected_index =
                    (state.library.selected_index + 1).min(visible_len - 1);
            }
            true
        }
        KeyCode::Up => {
            if !state.library.visible_indices().is_empty() {
                state.library.selected_index = state.library.selected_index.saturating_sub(1);
            }
            true
        }
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;
    use crate::library::Library;
    use crate::library::LibraryFilter;
    use crate::models::Chapter;
    use crate::models::book::BookStatus;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

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

    #[test]
    fn test_handle_library_i_opens_add_book() {
        let mut state = test_state();
        let (tx, _rx) = channel();
        let result = handle_library(&mut state, key_event(KeyCode::Char('i')), &tx);
        assert!(result);
        assert_eq!(state.current_page, Page::AddingBook);
        assert_eq!(
            state.modal,
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
        state.library.add_book("Test".into(), "url".into(), 10);
        state.library.books[0].chapters.push(Chapter {
            title: "Ch1".into(),
            url: "u1".into(),
            order: 0,
        });
        let (tx, _rx) = channel();
        let result = handle_library(&mut state, key_event(KeyCode::Char('j')), &tx);
        assert!(result);
        assert_eq!(state.current_page, Page::BookChapterJump);
        assert_eq!(
            state.modal,
            Modal::JumpChapter {
                chapters: vec![Chapter {
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
        state.library.add_book("A".into(), "url-a".into(), 10);
        state.library.add_book("B".into(), "url-b".into(), 20);
        let (tx, rx) = channel();
        let result = handle_library(&mut state, key_event(KeyCode::Char('u')), &tx);
        assert!(result);
        match rx.try_recv() {
            Ok(AppCommand::UpdateAll(urls)) => {
                assert_eq!(urls, vec![String::from("url-a"), String::from("url-b")]);
            }
            _ => panic!("Expected UpdateAll command"),
        }
    }

    #[test]
    fn test_handle_library_d_deletes() {
        let mut state = test_state();
        state.library.add_book("Test".into(), "url".into(), 10);
        assert_eq!(state.library.books.len(), 1);
        let (tx, _rx) = channel();
        let result = handle_library(&mut state, key_event(KeyCode::Char('d')), &tx);
        assert!(result);
        assert_eq!(state.library.books.len(), 0);
    }

    #[test]
    fn test_handle_library_f_cycles_filter() {
        let mut state = test_state();
        assert_eq!(state.library.filter, LibraryFilter::All);
        let (tx, _rx) = channel();
        let result = handle_library(&mut state, key_event(KeyCode::Char('f')), &tx);
        assert!(result);
        assert_eq!(
            state.library.filter,
            LibraryFilter::ByStatus(BookStatus::Reading)
        );
    }

    #[test]
    fn test_handle_library_space_cycles_status() {
        let mut state = test_state();
        state.library.add_book("Test".into(), "url".into(), 10);
        assert_eq!(state.library.books[0].status, BookStatus::Reading);
        let (tx, _rx) = channel();
        let result = handle_library(&mut state, key_event(KeyCode::Char(' ')), &tx);
        assert!(result);
        assert_eq!(state.library.books[0].status, BookStatus::Paused);
    }

    #[test]
    fn test_handle_library_enter_opens_session_picker() {
        let mut state = test_state();
        state.library.add_book("Test".into(), "url".into(), 10);
        let (tx, _rx) = channel();
        let result = handle_library(&mut state, key_event(KeyCode::Enter), &tx);
        assert!(result);
        assert_eq!(state.current_page, Page::Library);
        assert!(matches!(state.modal, Modal::SessionPicker { .. }));
    }

    #[test]
    fn test_handle_library_up_down_navigation() {
        let mut state = test_state();
        state.library.add_book("A".into(), "u1".into(), 10);
        state.library.add_book("B".into(), "u2".into(), 20);
        state.library.add_book("C".into(), "u3".into(), 30);
        let (tx, _rx) = channel();
        assert_eq!(state.library.selected_index, 0);
        handle_library(&mut state, key_event(KeyCode::Down), &tx);
        assert_eq!(state.library.selected_index, 1);
        handle_library(&mut state, key_event(KeyCode::Down), &tx);
        assert_eq!(state.library.selected_index, 2);
        handle_library(&mut state, key_event(KeyCode::Up), &tx);
        assert_eq!(state.library.selected_index, 1);
        handle_library(&mut state, key_event(KeyCode::Up), &tx);
        assert_eq!(state.library.selected_index, 0);
    }

    #[test]
    fn test_handle_library_up_does_not_go_below_zero() {
        let mut state = test_state();
        state.library.add_book("A".into(), "u1".into(), 10);
        let (tx, _rx) = channel();
        assert_eq!(state.library.selected_index, 0);
        handle_library(&mut state, key_event(KeyCode::Up), &tx);
        assert_eq!(state.library.selected_index, 0);
    }

    #[test]
    fn test_handle_library_down_does_not_exceed_max() {
        let mut state = test_state();
        state.library.add_book("A".into(), "u1".into(), 10);
        state.library.add_book("B".into(), "u2".into(), 20);
        let (tx, _rx) = channel();
        handle_library(&mut state, key_event(KeyCode::Down), &tx);
        handle_library(&mut state, key_event(KeyCode::Down), &tx);
        assert_eq!(state.library.selected_index, 1);
    }

    #[test]
    fn test_handle_library_unhandled_key_returns_true() {
        let mut state = test_state();
        let (tx, _rx) = channel();
        let result = handle_library(&mut state, key_event(KeyCode::Char('x')), &tx);
        assert!(result);
    }
}
