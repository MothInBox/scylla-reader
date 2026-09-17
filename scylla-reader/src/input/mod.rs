//! Input dispatch — routes crossterm events to page-specific handlers.

pub mod jobs;
pub mod keybinds;
pub mod library;
pub mod modal;
pub mod palette;
pub mod reader;
pub mod settings;

use crate::state::{AppState, Modal, Page};
use crossterm::event::KeyEvent;
use ratatui::prelude::Rect;

pub fn handle_input(state: &mut AppState, key: KeyEvent, size: Rect) -> bool {
    if matches!(&state.ui.modal, Modal::CommandPalette { .. }) {
        return palette::handle_palette(state, key);
    }
    if matches!(&state.ui.modal, Modal::SessionPicker { .. }) {
        return modal::handle_session_picker(state, key);
    }
    if matches!(&state.ui.modal, Modal::AddBook { .. }) {
        return modal::handle_adding_book(state, key);
    }
    if matches!(&state.ui.modal, Modal::JumpChapter { .. }) {
        return modal::handle_jumping_chapter(state, key);
    }
    if matches!(&state.ui.modal, Modal::InstallPlugin { .. }) {
        return modal::handle_installing_plugin(state, key);
    }
    if matches!(&state.ui.modal, Modal::BackendPicker { .. }) {
        return modal::handle_backend_picker(state, key);
    }
    if matches!(&state.ui.modal, Modal::ChapterResults { .. }) {
        return modal::handle_chapter_results(state, key);
    }
    if matches!(&state.ui.modal, Modal::EmbedChapters { .. }) {
        return modal::handle_embed_chapters(state, key);
    }
    if matches!(&state.ui.modal, Modal::Filter { .. }) {
        return modal::handle_filter(state, key);
    }
    match &state.ui.page {
        Page::Library => library::handle_library(&mut state.ui, &mut state.lib, key),
        Page::Settings => settings::handle_settings(&mut state.lib, key),
        Page::Reader => reader::handle_reader(state, key, size),
        Page::Jobs => jobs::handle_jobs(&mut state.jobs, &mut state.lib, key),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{Modal, Page};
    use crate::test_helpers::*;
    use crossterm::event::KeyCode;

    #[test]
    fn test_handle_input_adding_book_dispatches_correctly() {
        let mut state = test_state();
        state.ui.modal = Modal::AddBook {
            inputs: vec!["he".into()],
            cursor: 0,
            scroll_offset: 0,
        };
        let result = handle_input(&mut state, key_event(KeyCode::Char('l')), rect());
        assert!(result);
        if let Modal::AddBook { inputs, .. } = &state.ui.modal {
            assert_eq!(inputs[0], "hel");
        } else {
            panic!("Expected AddBook modal");
        }
    }

    #[test]
    fn test_handle_input_library_dispatches() {
        let mut state = test_state();
        state.ui.page = Page::Library;
        let result = handle_input(&mut state, key_event(KeyCode::Char('i')), rect());
        assert!(result);
        assert_eq!(state.ui.page, Page::Library);
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
    fn test_handle_input_settings_dispatches() {
        let mut state = test_state();
        state.ui.page = Page::Settings;
        let result = handle_input(&mut state, key_event(KeyCode::Enter), rect());
        assert!(result);
        assert!(state.lib.settings_ui.editing);
        assert_eq!(state.lib.settings_ui.edit_buffer, "2");
    }

    #[test]
    fn test_handle_input_reader_dispatches() {
        let mut state = test_state();
        state.lib.library.add_book("Test Book".into(), "url".into());
        state.ui.page = Page::Reader;
        let result = handle_input(&mut state, key_event(KeyCode::Right), rect());
        assert!(result);
    }

    #[test]
    fn test_handle_input_command_palette_takes_priority() {
        let mut state = test_state();
        state.ui.page = Page::Library;
        state.ui.modal = Modal::CommandPalette {
            query: String::new(),
            filtered: Vec::new(),
            selected: 0,
        };
        let result = handle_input(&mut state, key_event(KeyCode::Char('S')), rect());
        assert!(result);
        if let Modal::CommandPalette { query, .. } = &state.ui.modal {
            assert_eq!(query, "S");
        } else {
            panic!("Expected CommandPalette");
        }
        assert_eq!(state.ui.page, Page::Library);
    }
}
