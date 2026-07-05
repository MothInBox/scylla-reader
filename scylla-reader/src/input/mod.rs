//! Input dispatch — routes crossterm events to page-specific handlers.

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
    if matches!(&state.modal, Modal::SessionNameInput { .. }) {
        return modal::handle_session_name_input(state, key, cmd_tx);
    }
    match &state.current_page {
        Page::AddingBook => modal::handle_adding_book(state, key, cmd_tx),
        Page::Library => library::handle_library(state, key, cmd_tx),
        Page::Settings => settings::handle_settings(state, key, cmd_tx),
        Page::Reader => reader::handle_reader(state, key, cmd_tx, size),
        Page::BookChapterJump => modal::handle_jumping_chapter(state, key),
    }
}
