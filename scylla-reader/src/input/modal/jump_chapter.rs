use crate::input::keybinds::*;
use crate::messenger::AppCommand;
use crate::models::Chapter;
use crate::state::{AppState, Modal, Page};
use crossterm::event::{KeyCode, KeyEvent};
use std::sync::mpsc;

pub fn handle_jumping_chapter(
    state: &mut AppState,
    key: KeyEvent,
    _cmd_tx: &mpsc::Sender<AppCommand>,
) -> bool {
    let mut selected_cursor = None;

    if let Modal::JumpChapter {
        chapters,
        query,
        filtered,
        cursor,
        show_titles,
        ..
    } = &mut state.modal
    {
        match key.code {
            KEY_NAV_UP => {
                if *cursor > 0 {
                    *cursor -= 1;
                }
            }
            KEY_NAV_DOWN => {
                if *cursor < filtered.len().saturating_sub(1) {
                    *cursor += 1;
                }
            }
            KEY_ENTER => {
                selected_cursor = Some(*cursor);
            }
            KEY_TOGGLE_TITLES => {
                *show_titles = !*show_titles;
            }
            KeyCode::Char(c) => {
                query.push(c);
                *filtered = filter_chapters(chapters, query);
                *cursor = 0;
            }
            KEY_BACKSPACE => {
                query.pop();
                *filtered = filter_chapters(chapters, query);
                *cursor = 0;
            }
            _ => {}
        }
    }

    if let Some(cursor_val) = selected_cursor {
        let real_idx = if let Modal::JumpChapter {
            filtered, chapters, ..
        } = &state.modal
        {
            filtered
                .get(cursor_val)
                .and_then(|ch| chapters.iter().position(|c| c.url == ch.url))
        } else {
            None
        };

        if let Some(idx) = real_idx {
            crate::settings::log(
                crate::settings::LogLevel::Debug,
                "INPUT",
                &format!("Attempting jump: {}", idx),
            );
            if let Some(book) = state.library.selected_book_mut() {
                let active_id = book.active_session_id;
                if let Some(session) = book.sessions.iter_mut().find(|s| Some(s.id) == active_id) {
                    session.progress.current = idx as u32;
                    let _ = state
                        .db
                        .update_session_progress(session.id, session.progress.current);
                }
            }
        }
        state.modal = Modal::None;
        state.current_page = Page::Library;
    }
    true
}

fn filter_chapters(chapters: &[Chapter], query: &str) -> Vec<Chapter> {
    if query.trim().is_empty() {
        return chapters.to_vec();
    }
    let q = query.to_lowercase();
    chapters
        .iter()
        .filter(|ch| ch.title.to_lowercase().contains(&q) || ch.url.to_lowercase().contains(&q))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;
    use crate::library::Library;
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

    fn setup_jump_chapter_state(chapters: Vec<Chapter>, cursor: usize) -> AppState {
        let mut state = test_state();
        state.library.add_book("Test Book".into(), "url".into(), 10);
        if let Some(book) = state.library.selected_book_mut() {
            book.chapters = chapters.clone();
        }
        state.modal = Modal::JumpChapter {
            chapters: chapters.clone(),
            query: String::new(),
            filtered: chapters,
            cursor,
            scroll_offset: 0,
            show_titles: true,
        };
        state.current_page = Page::BookChapterJump;
        state
    }

    #[test]
    fn test_handle_jump_chapter_enter_selects_and_updates_progress() {
        let chapters = vec![
            Chapter {
                title: "Ch1".into(),
                url: "u1".into(),
                order: 0,
            },
            Chapter {
                title: "Ch2".into(),
                url: "u2".into(),
                order: 1,
            },
        ];
        let mut state = setup_jump_chapter_state(chapters, 1);
        state.reader.session_id = 0;
        if let Some(book) = state.library.selected_book_mut() {
            book.active_session_id = Some(0);
            book.sessions.push(crate::models::Session {
                id: 0,
                book_url: "url".into(),
                name: "default".into(),
                progress: crate::models::Progress {
                    current: 0,
                    total: 2,
                },
                created_at: String::new(),
                updated_at: String::new(),
            });
        }
        let (tx, _rx) = channel();
        let result = handle_jumping_chapter(&mut state, key_event(KEY_ENTER), &tx);
        assert!(result);
        assert_eq!(state.modal, Modal::None);
        assert_eq!(state.current_page, Page::Library);
        if let Some(book) = state.library.selected_book() {
            if let Some(session) = book.sessions.iter().find(|s| s.id == 0) {
                assert_eq!(session.progress.current, 1);
            } else {
                panic!("Expected session 0");
            }
        } else {
            panic!("Expected selected book");
        }
    }

    #[test]
    fn test_handle_jump_chapter_up_down_navigates() {
        let chapters = vec![
            Chapter {
                title: "Ch1".into(),
                url: "u1".into(),
                order: 0,
            },
            Chapter {
                title: "Ch2".into(),
                url: "u2".into(),
                order: 1,
            },
            Chapter {
                title: "Ch3".into(),
                url: "u3".into(),
                order: 2,
            },
        ];
        let mut state = setup_jump_chapter_state(chapters, 1);
        let (tx, _rx) = channel();
        handle_jumping_chapter(&mut state, key_event(KEY_NAV_UP), &tx);
        if let Modal::JumpChapter { cursor, .. } = &state.modal {
            assert_eq!(*cursor, 0);
        } else {
            panic!("Expected JumpChapter modal");
        }
        handle_jumping_chapter(&mut state, key_event(KEY_NAV_DOWN), &tx);
        if let Modal::JumpChapter { cursor, .. } = &state.modal {
            assert_eq!(*cursor, 1);
        } else {
            panic!("Expected JumpChapter modal");
        }
    }

    #[test]
    fn test_handle_jump_chapter_t_toggles_show_titles() {
        let chapters = vec![Chapter {
            title: "Ch1".into(),
            url: "u1".into(),
            order: 0,
        }];
        let mut state = setup_jump_chapter_state(chapters, 0);
        let (tx, _rx) = channel();
        handle_jumping_chapter(&mut state, key_event(KEY_TOGGLE_TITLES), &tx);
        if let Modal::JumpChapter { show_titles, .. } = &state.modal {
            assert!(!show_titles);
        } else {
            panic!("Expected JumpChapter modal");
        }
    }

    #[test]
    fn test_handle_jump_chapter_up_stays_at_top() {
        let chapters = vec![Chapter {
            title: "Ch1".into(),
            url: "u1".into(),
            order: 0,
        }];
        let mut state = setup_jump_chapter_state(chapters, 0);
        let (tx, _rx) = channel();
        handle_jumping_chapter(&mut state, key_event(KEY_NAV_UP), &tx);
        if let Modal::JumpChapter { cursor, .. } = &state.modal {
            assert_eq!(*cursor, 0);
        } else {
            panic!("Expected JumpChapter modal");
        }
    }

    #[test]
    fn test_handle_jump_chapter_down_stays_at_bottom() {
        let chapters = vec![Chapter {
            title: "Ch1".into(),
            url: "u1".into(),
            order: 0,
        }];
        let mut state = setup_jump_chapter_state(chapters, 0);
        let (tx, _rx) = channel();
        handle_jumping_chapter(&mut state, key_event(KEY_NAV_DOWN), &tx);
        if let Modal::JumpChapter { cursor, .. } = &state.modal {
            assert_eq!(*cursor, 0);
        } else {
            panic!("Expected JumpChapter modal");
        }
    }
}
