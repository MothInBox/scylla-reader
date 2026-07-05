//! Modal input handler — add-book form, jump-to-chapter list.

use crate::messenger::AppCommand;
use crate::state::{AppState, Modal, Page};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub fn handle_adding_book(
    state: &mut AppState,
    key: KeyEvent,
    cmd_tx: &std::sync::mpsc::Sender<AppCommand>,
) -> bool {
    if let (KeyModifiers::CONTROL, KeyCode::Char('s')) = (key.modifiers, key.code) {
        let urls: Vec<String> = if let Modal::AddBook { inputs, .. } = &state.modal {
            inputs
                .iter()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        } else {
            vec![]
        };

        if urls.is_empty() {
            crate::settings::log(
                crate::settings::LogLevel::Debug,
                "INPUT",
                "No valid URLs to scrape",
            );
        } else {
            crate::settings::log(
                crate::settings::LogLevel::Debug,
                "INPUT",
                &format!("Submitting {} URLs", urls.len()),
            );
            for url in urls {
                if let Err(e) = cmd_tx.send(AppCommand::Scrape(url)) {
                    crate::settings::log(
                        crate::settings::LogLevel::Error,
                        "INPUT",
                        &format!("Failed to queue scrape: {}", e),
                    );
                }
            }
        }

        state.modal = Modal::None;
        state.current_page = Page::Library;
        return true;
    }

    if let Modal::AddBook { inputs, cursor, .. } = &mut state.modal {
        match key.code {
            KeyCode::Enter => {
                inputs.insert(*cursor + 1, String::new());
                *cursor += 1;
            }
            KeyCode::Backspace => {
                let line_empty = inputs[*cursor].is_empty();
                if line_empty && inputs.len() > 1 {
                    inputs.remove(*cursor);
                    if *cursor > 0 {
                        *cursor -= 1;
                    }
                } else {
                    inputs[*cursor].pop();
                }
            }
            KeyCode::Up => {
                if *cursor > 0 {
                    *cursor -= 1;
                }
            }
            KeyCode::Down => {
                if *cursor < inputs.len().saturating_sub(1) {
                    *cursor += 1;
                }
            }
            KeyCode::Char(c)
                if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
            {
                inputs[*cursor].push(c);
            }
            _ => {}
        }
    }

    true
}

pub fn handle_jumping_chapter(state: &mut AppState, key: KeyEvent) -> bool {
    let mut selected_cursor = None;
    if let Modal::JumpChapter {
        chapters, cursor, ..
    } = &mut state.modal
    {
        match key.code {
            KeyCode::Up => {
                if *cursor > 0 {
                    *cursor -= 1;
                }
            }
            KeyCode::Down => {
                if *cursor < chapters.len().saturating_sub(1) {
                    *cursor += 1;
                }
            }
            KeyCode::Enter => {
                selected_cursor = Some(*cursor);
            }
            KeyCode::Char('t') => {
                if let Modal::JumpChapter { show_titles, .. } = &mut state.modal {
                    *show_titles = !*show_titles;
                }
            }
            _ => {}
        }
    }

    if let Some(cursor_val) = selected_cursor {
        if let Some(book) = state.library.selected_book_mut() {
            if let Some(session) = book
                .sessions
                .iter_mut()
                .find(|s| s.id == state.reader.session_id)
            {
                session.progress.current = cursor_val as u32;
                if let Err(err) =
                    state
                        .db
                        .update_session_progress(session.id, session.progress.current)
                {
                    crate::settings::log(
                        crate::settings::LogLevel::Debug,
                        "INPUT",
                        &format!("Failed to update session progress: {}", err),
                    );
                }
            }
        }
        state.modal = Modal::None;
        state.current_page = Page::Library;
    }

    true
}

pub fn handle_session_picker(
    state: &mut AppState,
    key: KeyEvent,
    cmd_tx: &std::sync::mpsc::Sender<AppCommand>,
) -> bool {
    let (book_url, cursor) = if let Modal::SessionPicker { book_url, cursor, .. } = &state.modal {
        (book_url.clone(), *cursor)
    } else {
        return true;
    };

    match key.code {
        KeyCode::Up => {
            if let Modal::SessionPicker { cursor, .. } = &mut state.modal {
                if *cursor > 0 {
                    *cursor -= 1;
                }
            }
        }
        KeyCode::Down => {
            if let Modal::SessionPicker { cursor, .. } = &mut state.modal {
                if let Some(book) = state.library.books.iter().find(|b| b.url == book_url) {
                    if *cursor < book.sessions.len().saturating_sub(1) {
                        *cursor += 1;
                    }
                }
            }
        }
        KeyCode::Enter => {
            let session = state.library.books.iter()
                .find(|b| b.url == book_url)
                .and_then(|b| b.sessions.get(cursor))
                .cloned();
            if let Some(session) = session {
                state.reader.session_id = session.id;
                state.reader.session_name = session.name.clone();
                if let Some(book) = state.library.books.iter_mut().find(|b| b.url == book_url) {
                    book.active_session_id = Some(session.id);
                }
                state.db.set_active_session(&book_url, Some(session.id)).ok();
                if let Some(book) = state.library.books.iter().find(|b| b.url == book_url) {
                    if !book.chapters.is_empty() {
                        let idx = (session.progress.current as usize).min(book.chapters.len() - 1);
                        state.reader.loading = true;
                        state.current_page = Page::Reader;
                        if let Some(ch) = book.chapters.get(idx) {
                            let _ = cmd_tx.send(AppCommand::FetchChapter(ch.url.clone(), idx));
                        }
                    }
                }
            }
            state.modal = Modal::None;
        }
        KeyCode::Char('n') => {
            state.modal = Modal::SessionNameInput {
                book_url: book_url.clone(),
                session_id: None,
                input: String::new(),
            };
        }
        KeyCode::Char('r') => {
            if let Some(book) = state.library.books.iter().find(|b| b.url == book_url) {
                if let Some(session) = book.sessions.get(cursor) {
                    state.modal = Modal::SessionNameInput {
                        book_url: book_url.clone(),
                        session_id: Some(session.id),
                        input: session.name.clone(),
                    };
                }
            }
        }
        KeyCode::Char('d') => {
            if let Some(book) = state.library.books.iter_mut().find(|b| b.url == book_url) {
                if book.sessions.len() > 1 && cursor < book.sessions.len() {
                    let session_id = book.sessions[cursor].id;
                    state.db.delete_session(session_id).ok();
                    if let Ok(loaded) = state.db.load_sessions_for_book(&book_url) {
                        book.sessions = loaded;
                    }
                    let new_cursor = cursor.min(book.sessions.len().saturating_sub(1));
                    if let Modal::SessionPicker { cursor: c, .. } = &mut state.modal {
                        *c = new_cursor;
                    }
                }
            }
        }
        _ => {}
    }

    true
}

pub fn handle_session_name_input(
    state: &mut AppState,
    key: KeyEvent,
    cmd_tx: &std::sync::mpsc::Sender<AppCommand>,
) -> bool {
    let (book_url, session_id, input) =
        if let Modal::SessionNameInput {
            book_url,
            session_id,
            input,
        } = &state.modal
        {
            (book_url.clone(), *session_id, input.clone())
        } else {
            return true;
        };

    match key.code {
        KeyCode::Esc => {
            state.modal = Modal::SessionPicker {
                book_url: book_url.clone(),
                cursor: 0,
                scroll_offset: 0,
            };
        }
        KeyCode::Enter => {
            let name = input.trim().to_string();
            if name.is_empty() {
                return true;
            }
            if let Some(session_id) = session_id {
                if let Some(book) = state.library.books.iter_mut().find(|b| b.url == book_url) {
                    if let Some(session) = book.sessions.iter_mut().find(|s| s.id == session_id) {
                        session.name = name.clone();
                    }
                }
                state.db.rename_session(session_id, &name).ok();
            } else {
                if let Some(book) = state.library.books.iter_mut().find(|b| b.url == book_url) {
                    if let Ok(session) =
                        state
                            .db
                            .create_session(&book_url, &name, book.chapters.len() as u32)
                    {
                        if let Ok(loaded) = state.db.load_sessions_for_book(&book_url) {
                            book.sessions = loaded;
                        }
                        if let Some(s) = book.sessions.iter().find(|s| s.id == session.id) {
                            book.active_session_id = Some(s.id);
                            state.reader.session_id = s.id;
                            state.reader.session_name = s.name.clone();
                            state.db.set_active_session(&book_url, Some(s.id)).ok();
                        }
                        if !book.chapters.is_empty() {
                            state.reader.loading = true;
                            state.current_page = Page::Reader;
                            if let Some(ch) = book.chapters.first() {
                                let _ = cmd_tx.send(AppCommand::FetchChapter(ch.url.clone(), 0));
                            }
                        }
                    }
                }
            }
            state.modal = Modal::None;
        }
        KeyCode::Backspace => {
            if let Modal::SessionNameInput { input, .. } = &mut state.modal {
                input.pop();
            }
        }
        KeyCode::Char(c) => {
            if let Modal::SessionNameInput { input, .. } = &mut state.modal {
                input.push(c);
            }
        }
        _ => {}
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;
    use crate::library::Library;
    use crate::models::Chapter;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn test_state() -> AppState {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let db = Db::open_conn(conn).unwrap();
        AppState::from_parts(db, Library::new())
    }

    fn key_event(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn key_event_ctrl_s() -> KeyEvent {
        KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)
    }

    fn channel() -> (
        std::sync::mpsc::Sender<AppCommand>,
        std::sync::mpsc::Receiver<AppCommand>,
    ) {
        std::sync::mpsc::channel()
    }

    fn setup_add_book_state(inputs: Vec<String>, cursor: usize) -> AppState {
        let mut state = test_state();
        state.modal = Modal::AddBook {
            inputs,
            cursor,
            scroll_offset: 0,
        };
        state.current_page = Page::AddingBook;
        state
    }

    fn setup_jump_chapter_state(chapters: Vec<Chapter>, cursor: usize) -> AppState {
        let mut state = test_state();
        state.library.add_book("Test Book".into(), "url".into(), 10);
        if let Some(book) = state.library.selected_book_mut() {
            book.chapters = chapters.clone();
        }
        state.modal = Modal::JumpChapter {
            chapters,
            cursor,
            scroll_offset: 0,
            show_titles: true,
        };
        state.current_page = Page::BookChapterJump;
        state
    }

    #[test]
    fn test_handle_adding_book_ctrl_s_submits() {
        let mut state = setup_add_book_state(
            vec!["http://example.com".into(), "http://test.org".into()],
            0,
        );
        let (tx, rx) = channel();
        let result = handle_adding_book(&mut state, key_event_ctrl_s(), &tx);
        assert!(result);
        assert_eq!(state.modal, Modal::None);
        assert_eq!(state.current_page, Page::Library);

        let received: Vec<AppCommand> = rx.try_iter().collect();
        assert_eq!(received.len(), 2);
        assert!(matches!(&received[0], AppCommand::Scrape(url) if url == "http://example.com"));
        assert!(matches!(&received[1], AppCommand::Scrape(url) if url == "http://test.org"));
    }

    #[test]
    fn test_handle_adding_book_ctrl_s_empty_inputs_does_not_submit() {
        let mut state = setup_add_book_state(vec!["".into(), "  ".into()], 0);
        let (tx, rx) = channel();
        let result = handle_adding_book(&mut state, key_event_ctrl_s(), &tx);
        assert!(result);
        assert_eq!(state.modal, Modal::None);
        assert_eq!(state.current_page, Page::Library);

        let received: Vec<AppCommand> = rx.try_iter().collect();
        assert!(received.is_empty());
    }

    #[test]
    fn test_handle_adding_book_enter_adds_new_line() {
        let mut state = setup_add_book_state(vec!["line1".into(), "line2".into()], 0);
        let (tx, _rx) = channel();
        let result = handle_adding_book(&mut state, key_event(KeyCode::Enter), &tx);
        assert!(result);
        if let Modal::AddBook { inputs, cursor, .. } = &state.modal {
            assert_eq!(inputs.len(), 3);
            assert_eq!(inputs[0], "line1");
            assert_eq!(inputs[1], "");
            assert_eq!(inputs[2], "line2");
            assert_eq!(*cursor, 1);
        } else {
            panic!("Expected AddBook modal");
        }
    }

    #[test]
    fn test_handle_adding_book_backspace_deletes_empty_line() {
        let mut state = setup_add_book_state(vec!["a".into(), "".into(), "b".into()], 1);
        let (tx, _rx) = channel();
        let result = handle_adding_book(&mut state, key_event(KeyCode::Backspace), &tx);
        assert!(result);
        if let Modal::AddBook { inputs, cursor, .. } = &state.modal {
            assert_eq!(inputs.len(), 2);
            assert_eq!(inputs[0], "a");
            assert_eq!(inputs[1], "b");
            assert_eq!(*cursor, 0);
        } else {
            panic!("Expected AddBook modal");
        }
    }

    #[test]
    fn test_handle_adding_book_backspace_pops_char_on_non_empty_line() {
        let mut state = setup_add_book_state(vec!["hello".into()], 0);
        let (tx, _rx) = channel();
        let result = handle_adding_book(&mut state, key_event(KeyCode::Backspace), &tx);
        assert!(result);
        if let Modal::AddBook { inputs, .. } = &state.modal {
            assert_eq!(inputs[0], "hell");
        } else {
            panic!("Expected AddBook modal");
        }
    }

    #[test]
    fn test_handle_adding_book_up_down_navigation() {
        let mut state = setup_add_book_state(vec!["a".into(), "b".into(), "c".into()], 1);
        let (tx, _rx) = channel();
        handle_adding_book(&mut state, key_event(KeyCode::Up), &tx);
        if let Modal::AddBook { cursor, .. } = &state.modal {
            assert_eq!(*cursor, 0);
        } else {
            panic!("Expected AddBook modal");
        }
        handle_adding_book(&mut state, key_event(KeyCode::Down), &tx);
        if let Modal::AddBook { cursor, .. } = &state.modal {
            assert_eq!(*cursor, 1);
        } else {
            panic!("Expected AddBook modal");
        }
    }

    #[test]
    fn test_handle_adding_book_up_stays_at_top() {
        let mut state = setup_add_book_state(vec!["a".into(), "b".into()], 0);
        let (tx, _rx) = channel();
        handle_adding_book(&mut state, key_event(KeyCode::Up), &tx);
        if let Modal::AddBook { cursor, .. } = &state.modal {
            assert_eq!(*cursor, 0);
        } else {
            panic!("Expected AddBook modal");
        }
    }

    #[test]
    fn test_handle_adding_book_down_stays_at_bottom() {
        let mut state = setup_add_book_state(vec!["a".into(), "b".into()], 1);
        let (tx, _rx) = channel();
        handle_adding_book(&mut state, key_event(KeyCode::Down), &tx);
        if let Modal::AddBook { cursor, .. } = &state.modal {
            assert_eq!(*cursor, 1);
        } else {
            panic!("Expected AddBook modal");
        }
    }

    #[test]
    fn test_handle_adding_book_types_characters() {
        let mut state = setup_add_book_state(vec!["he".into()], 0);
        let (tx, _rx) = channel();
        handle_adding_book(&mut state, key_event(KeyCode::Char('l')), &tx);
        handle_adding_book(&mut state, key_event(KeyCode::Char('o')), &tx);
        if let Modal::AddBook { inputs, .. } = &state.modal {
            assert_eq!(inputs[0], "helo");
        } else {
            panic!("Expected AddBook modal");
        }
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
                progress: crate::models::Progress { current: 0, total: 2 },
                created_at: String::new(),
                updated_at: String::new(),
            });
        }
        let result = handle_jumping_chapter(&mut state, key_event(KeyCode::Enter));
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
        handle_jumping_chapter(&mut state, key_event(KeyCode::Up));
        if let Modal::JumpChapter { cursor, .. } = &state.modal {
            assert_eq!(*cursor, 0);
        } else {
            panic!("Expected JumpChapter modal");
        }
        handle_jumping_chapter(&mut state, key_event(KeyCode::Down));
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
        handle_jumping_chapter(&mut state, key_event(KeyCode::Char('t')));
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
        handle_jumping_chapter(&mut state, key_event(KeyCode::Up));
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
        handle_jumping_chapter(&mut state, key_event(KeyCode::Down));
        if let Modal::JumpChapter { cursor, .. } = &state.modal {
            assert_eq!(*cursor, 0);
        } else {
            panic!("Expected JumpChapter modal");
        }
    }
}
