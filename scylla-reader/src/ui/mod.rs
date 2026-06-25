pub mod library;
pub mod modal;
pub mod reader;
pub mod settings;
pub mod widgets;

use crate::app::AppState;
use crate::app::page::Page;
use ratatui::prelude::*;

pub fn draw(frame: &mut Frame, state: &mut AppState, area: Rect) {
    match state.current_page {
        Page::Library | Page::AddingBook | Page::BookChapterJump => {
            library::draw(frame, area, state);
        }
        Page::Settings => {
            settings::draw(frame, area, state);
        }
        Page::Reader => {
            reader::draw(frame, area, state);
        }
    }

    modal::draw_modal(frame, area, state);
}
