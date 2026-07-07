use crate::input::keybinds::*;
use crate::messenger::AppCommand;
use crate::state::{AppState, Modal, Page};
use crossterm::event::{KeyCode, KeyEvent};

pub fn handle_session_picker(
    state: &mut AppState,
    key: KeyEvent,
    cmd_tx: &std::sync::mpsc::Sender<AppCommand>,
) -> bool {
    let (book_url, cursor, is_editing) = if let Modal::SessionPicker {
        book_url,
        cursor,
        input,
        ..
    } = &state.modal
    {
        (book_url.clone(), *cursor, input.is_some())
    } else {
        return true;
    };

    if is_editing {
        return handle_session_picker_editing(state, key, cmd_tx);
    }

    match key.code {
        KEY_NAV_UP => {
            if let Modal::SessionPicker { cursor, .. } = &mut state.modal {
                if *cursor > 0 {
                    *cursor -= 1;
                }
            }
        }
        KEY_NAV_DOWN => {
            if let Modal::SessionPicker { cursor, .. } = &mut state.modal {
                if let Some(book) = state.library.books.iter().find(|b| b.url == book_url) {
                    if *cursor < book.sessions.len().saturating_sub(1) {
                        *cursor += 1;
                    }
                }
            }
        }
        KEY_ESCAPE => {
            if let Modal::SessionPicker {
                pending_delete_url, ..
            } = &mut state.modal
            {
                *pending_delete_url = None;
            }
            state.modal = Modal::None;
        }
        KEY_ENTER => {
            if let Modal::SessionPicker {
                pending_delete_url, ..
            } = &mut state.modal
            {
                *pending_delete_url = None;
            }
            let session = state
                .library
                .books
                .iter()
                .find(|b| b.url == book_url)
                .and_then(|b| b.sessions.get(cursor))
                .cloned();
            if let Some(session) = session {
                state.reader.session_id = session.id;
                state.reader.session_name = session.name.clone();
                if let Some(book) = state.library.books.iter_mut().find(|b| b.url == book_url) {
                    book.active_session_id = Some(session.id);
                }
                state
                    .db
                    .set_active_session(&book_url, Some(session.id))
                    .ok();
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
        KEY_NEW_SESSION => {
            if let Modal::SessionPicker {
                cursor,
                input,
                editing_id,
                pending_delete_url,
                ..
            } = &mut state.modal
            {
                *cursor = 0;
                *input = Some(String::new());
                *editing_id = None;
                *pending_delete_url = None;
            }
        }
        KEY_RENAME_SESSION => {
            if let Modal::SessionPicker {
                pending_delete_url, ..
            } = &mut state.modal
            {
                *pending_delete_url = None;
            }
            if let Some(book) = state.library.books.iter().find(|b| b.url == book_url) {
                if let Some(session) = book.sessions.get(cursor) {
                    if let Modal::SessionPicker {
                        input, editing_id, ..
                    } = &mut state.modal
                    {
                        *input = Some(session.name.clone());
                        *editing_id = Some(session.id);
                    }
                }
            }
        }
        KEY_DELETE_SESSION => {
            let session_count = state
                .library
                .books
                .iter()
                .find(|b| b.url == book_url)
                .map(|b| b.sessions.len())
                .unwrap_or(0);
            if cursor < session_count {
                let is_last_session = session_count == 1;
                if is_last_session {
                    if let Modal::SessionPicker {
                        pending_delete_url, ..
                    } = &mut state.modal
                    {
                        if pending_delete_url.is_none() {
                            *pending_delete_url = Some(book_url.clone());
                            return true;
                        }
                        *pending_delete_url = None;
                    }
                }
                let session_id = state
                    .library
                    .books
                    .iter()
                    .find(|b| b.url == book_url)
                    .and_then(|b| b.sessions.get(cursor))
                    .map(|s| s.id);
                if let Some(session_id) = session_id {
                    if session_count > 1 {
                        state.db.delete_session(session_id).ok();
                        if let Ok(loaded) = state.db.load_sessions_for_book(&book_url) {
                            if let Some(book) =
                                state.library.books.iter_mut().find(|b| b.url == book_url)
                            {
                                book.sessions = loaded;
                            }
                        }
                        let new_cursor = cursor.min(session_count.saturating_sub(2));
                        if let Modal::SessionPicker { cursor: c, .. } = &mut state.modal {
                            *c = new_cursor;
                        }
                    } else {
                        state.db.delete_book(&book_url).ok();
                        state.library.books.retain(|b| b.url != book_url);
                        let new_len = state.library.visible_indices().len();
                        if state.library.selected_index > 0
                            && state.library.selected_index >= new_len
                        {
                            state.library.selected_index -= 1;
                        }
                        state.modal = Modal::None;
                    }
                }
            }
        }
        _ => {
            if let Modal::SessionPicker {
                pending_delete_url, ..
            } = &mut state.modal
            {
                *pending_delete_url = None;
            }
        }
    }

    true
}

fn handle_session_picker_editing(
    state: &mut AppState,
    key: KeyEvent,
    cmd_tx: &std::sync::mpsc::Sender<AppCommand>,
) -> bool {
    let (book_url, editing_id, input) = if let Modal::SessionPicker {
        book_url,
        editing_id,
        input,
        ..
    } = &state.modal
    {
        (
            book_url.clone(),
            *editing_id,
            input.clone().unwrap_or_default(),
        )
    } else {
        return true;
    };

    match key.code {
        KEY_ESCAPE => {
            if let Modal::SessionPicker {
                input, editing_id, ..
            } = &mut state.modal
            {
                *input = None;
                *editing_id = None;
            }
        }
        KEY_ENTER => {
            let name = input.trim().to_string();
            if name.is_empty() {
                return true;
            }
            if let Some(session_id) = editing_id {
                if let Some(book) = state.library.books.iter_mut().find(|b| b.url == book_url) {
                    if let Some(session) = book.sessions.iter_mut().find(|s| s.id == session_id) {
                        session.name = name.clone();
                    }
                }
                state.db.rename_session(session_id, &name).ok();
                if let Ok(loaded) = state.db.load_sessions_for_book(&book_url) {
                    if let Some(book) = state.library.books.iter_mut().find(|b| b.url == book_url) {
                        book.sessions = loaded;
                    }
                }
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
            if let Modal::SessionPicker {
                input,
                editing_id,
                cursor,
                ..
            } = &mut state.modal
            {
                *input = None;
                *editing_id = None;
                *cursor = 0;
            }
        }
        KEY_BACKSPACE => {
            if let Modal::SessionPicker { input, .. } = &mut state.modal {
                if let Some(text) = input {
                    text.pop();
                }
            }
        }
        KeyCode::Char(c) => {
            if let Modal::SessionPicker { input, .. } = &mut state.modal {
                if let Some(text) = input {
                    text.push(c);
                }
            }
        }
        _ => {}
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyCode;
    use crate::test_helpers::*;

    fn setup_session_picker_state(sessions: usize, cursor: usize) -> AppState {
        let mut state = test_state();
        state.library.add_book("Test Book".into(), "test_url".into());
        if let Some(book) = state.library.books.iter().find(|b| b.url == "test_url") {
            let _ = state.db.upsert_book(book);
        }
        // Remove the default "Initial" session that upsert_book created
        if let Ok(loaded) = state.db.load_sessions_for_book("test_url") {
            for s in &loaded {
                let _ = state.db.delete_session(s.id);
            }
        }
        // Create the exact number of test sessions
        for i in 0..sessions {
            let _ = state
                .db
                .create_session("test_url", &format!("Session {}", i), 10);
        }
        // Reload sessions from DB into library model
        if let Ok(loaded) = state.db.load_sessions_for_book("test_url") {
            if let Some(book) = state.library.books.iter_mut().find(|b| b.url == "test_url") {
                book.sessions = loaded;
            }
        }
        state.modal = Modal::SessionPicker {
            book_url: "test_url".into(),
            cursor,
            scroll_offset: 0,
            input: None,
            editing_id: None,
            pending_delete_url: None,
        };
        state
    }

    #[test]
    fn test_session_picker_up_down_navigation() {
        let mut state = setup_session_picker_state(3, 1);
        let (tx, _rx) = channel();
        handle_session_picker(&mut state, key_event(KEY_NAV_UP), &tx);
        if let Modal::SessionPicker { cursor, .. } = &state.modal {
            assert_eq!(*cursor, 0);
        }
        handle_session_picker(&mut state, key_event(KEY_NAV_DOWN), &tx);
        if let Modal::SessionPicker { cursor, .. } = &state.modal {
            assert_eq!(*cursor, 1);
        }
    }

    #[test]
    fn test_session_picker_esc_closes() {
        let mut state = setup_session_picker_state(2, 0);
        let (tx, _rx) = channel();
        handle_session_picker(&mut state, key_event(KEY_ESCAPE), &tx);
        assert_eq!(state.modal, Modal::None);
    }

    #[test]
    fn test_session_picker_n_opens_input() {
        let mut state = setup_session_picker_state(1, 0);
        let (tx, _rx) = channel();
        handle_session_picker(&mut state, key_event(KEY_NEW_SESSION), &tx);
        if let Modal::SessionPicker { input, .. } = &state.modal {
            assert!(input.is_some());
        } else {
            panic!("Expected SessionPicker modal");
        }
    }

    #[test]
    fn test_session_picker_delete_non_last_session() {
        let mut state = setup_session_picker_state(3, 0);
        let (tx, _rx) = channel();
        handle_session_picker(&mut state, key_event(KEY_DELETE_SESSION), &tx);
        if let Modal::SessionPicker { cursor, .. } = &state.modal {
            assert_eq!(*cursor, 0);
            if let Some(book) = state.library.books.iter().find(|b| b.url == "test_url") {
                assert_eq!(book.sessions.len(), 2);
            } else {
                panic!("Expected book");
            }
        } else {
            panic!("Expected SessionPicker modal");
        }
    }

    #[test]
    fn test_session_picker_delete_last_session_requires_confirmation() {
        let mut state = setup_session_picker_state(1, 0);
        let (tx, _rx) = channel();
        // First 'd' press: sets pending_delete_url, doesn't delete
        handle_session_picker(&mut state, key_event(KEY_DELETE_SESSION), &tx);
        if let Modal::SessionPicker {
            pending_delete_url, ..
        } = &state.modal
        {
            assert_eq!(
                pending_delete_url.as_deref(),
                Some("test_url"),
                "pending_delete_url should be set"
            );
        } else {
            panic!("Expected SessionPicker modal");
        }
        // Book should still exist
        assert_eq!(state.library.books.len(), 1);
        assert!(!matches!(state.modal, Modal::None));
    }

    #[test]
    fn test_session_picker_delete_last_session_confirmed() {
        let mut state = setup_session_picker_state(1, 0);
        // Set pending_delete_url to simulate first press
        if let Modal::SessionPicker {
            pending_delete_url, ..
        } = &mut state.modal
        {
            *pending_delete_url = Some("test_url".into());
        }
        let (tx, _rx) = channel();
        // Second 'd' press: confirms deletion
        handle_session_picker(&mut state, key_event(KEY_DELETE_SESSION), &tx);
        assert_eq!(state.library.books.len(), 0, "Book should be deleted");
        assert_eq!(state.modal, Modal::None, "Modal should be closed");
    }

    #[test]
    fn test_session_picker_other_key_clears_pending_delete() {
        let mut state = setup_session_picker_state(1, 0);
        if let Modal::SessionPicker {
            pending_delete_url, ..
        } = &mut state.modal
        {
            *pending_delete_url = Some("test_url".into());
        }
        let (tx, _rx) = channel();
        // Press some other key (e.g., 'x')
        handle_session_picker(&mut state, key_event(KeyCode::Char('x')), &tx);
        if let Modal::SessionPicker {
            pending_delete_url, ..
        } = &state.modal
        {
            assert!(
                pending_delete_url.is_none(),
                "pending_delete_url should be cleared"
            );
        } else {
            panic!("Expected SessionPicker modal");
        }
        // Book should still exist
        assert_eq!(state.library.books.len(), 1);
    }

    #[test]
    fn test_session_picker_delete_last_session_esc_cancels() {
        let mut state = setup_session_picker_state(1, 0);
        if let Modal::SessionPicker {
            pending_delete_url, ..
        } = &mut state.modal
        {
            *pending_delete_url = Some("test_url".into());
        }
        let (tx, _rx) = channel();
        // Esc should close the modal entirely (it already clears pending_delete_url in the Esc handler)
        handle_session_picker(&mut state, key_event(KEY_ESCAPE), &tx);
        assert_eq!(state.modal, Modal::None);
    }

    #[test]
    fn test_session_picker_enter_selects_session() {
        let mut state = setup_session_picker_state(2, 0);
        let (tx, _rx) = channel();
        handle_session_picker(&mut state, key_event(KEY_ENTER), &tx);
        assert_eq!(state.modal, Modal::None);
        assert_eq!(state.reader.session_name, "Session 0");
    }
}
