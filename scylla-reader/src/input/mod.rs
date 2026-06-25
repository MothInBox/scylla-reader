pub mod library;
pub mod modal;
pub mod reader;
pub mod settings;

use crate::messenger::AppCommand;
use crate::state::{AppState, Page};
use crossterm::event::KeyEvent;
use ratatui::prelude::Rect;

pub fn handle_input(
    state: &mut AppState,
    key: KeyEvent,
    cmd_tx: &std::sync::mpsc::Sender<AppCommand>,
    size: Rect,
) -> bool {
    match state.current_page.clone() {
        Page::AddingBook => modal::handle_adding_book(state, key, cmd_tx),
        Page::Library => library::handle_library(state, key, cmd_tx),
        Page::Settings => settings::handle_settings(state, key),
        Page::Reader => reader::handle_reader(state, key, cmd_tx, size),
        Page::BookChapterJump => modal::handle_jumping_chapter(state, key),
    }
}
