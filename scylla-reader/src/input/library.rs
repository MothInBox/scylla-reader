//! Library page input handler — navigation, filter, add/delete, jump.

use crate::input::keybinds::*;
use crate::messenger::AppCommand;
use crate::state::{LibraryState, Modal, UiState};
use crossterm::event::KeyEvent;

pub fn handle_library(
    ui: &mut UiState,
    lib: &mut LibraryState,
    key: KeyEvent,
    cmd_tx: &std::sync::mpsc::Sender<AppCommand>,
) -> bool {
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
        KEY_DELETE => {
            lib.library.remove_selected();
            true
        }
        KEY_CYCLE_STATUS => {
            lib.library.cycle_selected_status();
            true
        }
        KEY_CYCLE_FILTER => {
            lib.library.cycle_filter();
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
    use crate::library::LibraryFilter;
    use crate::models::Chapter;
    use crate::models::book::BookStatus;
    use crate::state::Page;
    use crate::test_helpers::*;
    use crossterm::event::KeyCode;

    #[test]
    fn test_handle_library_i_opens_add_book() {
        let mut state = test_state();
        let (tx, _rx) = channel();
        let result = handle_library(&mut state.ui, &mut state.lib, key_event(KEY_ADD_BOOK), &tx);
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
        let (tx, _rx) = channel();
        let result = handle_library(
            &mut state.ui,
            &mut state.lib,
            key_event(KEY_JUMP_CHAPTER),
            &tx,
        );
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
        let (tx, rx) = channel();
        let result = handle_library(
            &mut state.ui,
            &mut state.lib,
            key_event(KEY_UPDATE_ALL),
            &tx,
        );
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
        state.lib.library.add_book("Test".into(), "url".into());
        assert_eq!(state.lib.library.books.len(), 1);
        let (tx, _rx) = channel();
        let result = handle_library(&mut state.ui, &mut state.lib, key_event(KEY_DELETE), &tx);
        assert!(result);
        assert_eq!(state.lib.library.books.len(), 0);
    }

    #[test]
    fn test_handle_library_f_cycles_filter() {
        let mut state = test_state();
        assert_eq!(state.lib.library.filter, LibraryFilter::All);
        let (tx, _rx) = channel();
        let result = handle_library(
            &mut state.ui,
            &mut state.lib,
            key_event(KEY_CYCLE_FILTER),
            &tx,
        );
        assert!(result);
        assert_eq!(
            state.lib.library.filter,
            LibraryFilter::ByStatus(BookStatus::Reading)
        );
    }

    #[test]
    fn test_handle_library_space_cycles_status() {
        let mut state = test_state();
        state.lib.library.add_book("Test".into(), "url".into());
        assert_eq!(state.lib.library.books[0].status, BookStatus::Reading);
        let (tx, _rx) = channel();
        let result = handle_library(
            &mut state.ui,
            &mut state.lib,
            key_event(KEY_CYCLE_STATUS),
            &tx,
        );
        assert!(result);
        assert_eq!(state.lib.library.books[0].status, BookStatus::Paused);
    }

    #[test]
    fn test_handle_library_enter_opens_session_picker() {
        let mut state = test_state();
        state.lib.library.add_book("Test".into(), "url".into());
        let (tx, _rx) = channel();
        let result = handle_library(&mut state.ui, &mut state.lib, key_event(KEY_SESSIONS), &tx);
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
        let (tx, _rx) = channel();
        assert_eq!(state.lib.library.selected_index, 0);
        handle_library(&mut state.ui, &mut state.lib, key_event(KEY_NAV_DOWN), &tx);
        assert_eq!(state.lib.library.selected_index, 1);
        handle_library(&mut state.ui, &mut state.lib, key_event(KEY_NAV_DOWN), &tx);
        assert_eq!(state.lib.library.selected_index, 2);
        handle_library(&mut state.ui, &mut state.lib, key_event(KEY_NAV_UP), &tx);
        assert_eq!(state.lib.library.selected_index, 1);
        handle_library(&mut state.ui, &mut state.lib, key_event(KEY_NAV_UP), &tx);
        assert_eq!(state.lib.library.selected_index, 0);
    }

    #[test]
    fn test_handle_library_up_does_not_go_below_zero() {
        let mut state = test_state();
        state.lib.library.add_book("A".into(), "u1".into());
        let (tx, _rx) = channel();
        assert_eq!(state.lib.library.selected_index, 0);
        handle_library(&mut state.ui, &mut state.lib, key_event(KEY_NAV_UP), &tx);
        assert_eq!(state.lib.library.selected_index, 0);
    }

    #[test]
    fn test_handle_library_down_does_not_exceed_max() {
        let mut state = test_state();
        state.lib.library.add_book("A".into(), "u1".into());
        state.lib.library.add_book("B".into(), "u2".into());
        let (tx, _rx) = channel();
        handle_library(&mut state.ui, &mut state.lib, key_event(KEY_NAV_DOWN), &tx);
        handle_library(&mut state.ui, &mut state.lib, key_event(KEY_NAV_DOWN), &tx);
        assert_eq!(state.lib.library.selected_index, 1);
    }

    #[test]
    fn test_handle_library_unhandled_key_returns_true() {
        let mut state = test_state();
        let (tx, _rx) = channel();
        let result = handle_library(
            &mut state.ui,
            &mut state.lib,
            key_event(KeyCode::Char('x')),
            &tx,
        );
        assert!(result);
    }
}
