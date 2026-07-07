//! Input dispatch — routes crossterm events to page-specific handlers.

pub mod jobs;
pub mod keybinds;
pub mod library;
pub mod modal;
pub mod palette;
pub mod reader;
pub mod settings;

use crate::messenger::AppCommand;
use crate::state::{AppState, Modal, Page};
use crossterm::event::KeyEvent;
use ratatui::prelude::Rect;

pub fn handle_input(
    state: &mut AppState,
    key: KeyEvent,
    cmd_tx: &std::sync::mpsc::Sender<AppCommand>,
    size: Rect,
) -> bool {
    if matches!(&state.modal, Modal::CommandPalette { .. }) {
        return palette::handle_palette(state, key, cmd_tx);
    }
    if matches!(&state.modal, Modal::SessionPicker { .. }) {
        return modal::handle_session_picker(state, key, cmd_tx);
    }
    match &state.current_page {
        Page::AddingBook => modal::handle_adding_book(state, key, cmd_tx),
        Page::Library => library::handle_library(state, key, cmd_tx),
        Page::Settings => settings::handle_settings(state, key, cmd_tx),
        Page::Reader => reader::handle_reader(state, key, cmd_tx, size),
        Page::BookChapterJump => modal::handle_jumping_chapter(state, key, cmd_tx),
        Page::Jobs => jobs::handle_jobs(state, key, cmd_tx),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{Modal, Page};
    use crossterm::event::KeyCode;
    use crate::test_helpers::*;

    #[test]
    fn test_handle_input_adding_book_dispatches_correctly() {
        let mut state = test_state();
        state.current_page = Page::AddingBook;
        state.modal = Modal::AddBook {
            inputs: vec!["he".into()],
            cursor: 0,
            scroll_offset: 0,
        };
        let (tx, _rx) = channel();
        let result = handle_input(&mut state, key_event(KeyCode::Char('l')), &tx, rect());
        assert!(result);
        if let Modal::AddBook { inputs, .. } = &state.modal {
            assert_eq!(inputs[0], "hel");
        } else {
            panic!("Expected AddBook modal");
        }
    }

    #[test]
    fn test_handle_input_library_dispatches() {
        let mut state = test_state();
        state.current_page = Page::Library;
        let (tx, _rx) = channel();
        let result = handle_input(&mut state, key_event(KeyCode::Char('i')), &tx, rect());
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
    fn test_handle_input_settings_dispatches() {
        let mut state = test_state();
        state.current_page = Page::Settings;
        let (tx, _rx) = channel();
        let result = handle_input(&mut state, key_event(KeyCode::Enter), &tx, rect());
        assert!(result);
        assert!(state.settings_ui.editing);
        assert_eq!(state.settings_ui.edit_buffer, "2");
    }

    #[test]
    fn test_handle_input_reader_dispatches() {
        let mut state = test_state();
        state.library.add_book("Test Book".into(), "url".into());
        state.current_page = Page::Reader;
        let (tx, _rx) = channel();
        let result = handle_input(&mut state, key_event(KeyCode::Right), &tx, rect());
        assert!(result);
    }

    #[test]
    fn test_handle_input_command_palette_takes_priority() {
        let mut state = test_state();
        state.current_page = Page::Library;
        state.modal = Modal::CommandPalette {
            query: String::new(),
            filtered: Vec::new(),
            selected: 0,
        };
        let (tx, _rx) = channel();
        let result = handle_input(&mut state, key_event(KeyCode::Char('S')), &tx, rect());
        assert!(result);
        if let Modal::CommandPalette { query, .. } = &state.modal {
            assert_eq!(query, "S");
        } else {
            panic!("Expected CommandPalette");
        }
        assert_eq!(state.current_page, Page::Library);
    }
}
