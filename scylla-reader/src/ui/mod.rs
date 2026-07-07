//! Render dispatch — routes by current page to page-specific draw functions.

pub mod library;
pub mod modal;
pub mod palette;
pub mod reader;
pub mod settings;
pub mod jobs;
pub mod widgets;

use crate::state::AppState;
use crate::state::page::Page;
use ratatui::prelude::*;

pub fn draw(frame: &mut Frame, state: &mut AppState, area: Rect) {
    match state.current_page {
        Page::Library | Page::AddingBook | Page::BookChapterJump | Page::InstallingPlugin => {
            library::draw(frame, area, state);
        }
        Page::Settings => {
            settings::draw(frame, area, state);
        }
        Page::Reader => {
            reader::draw(frame, area, state);
        }
        Page::Jobs => {
            jobs::draw(frame, area, state);
        }
    }

    modal::draw_modal(frame, area, state);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Page;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use crate::test_helpers::*;

    #[test]
    fn test_draw_library_does_not_panic() {
        let mut state = test_state();
        state.current_page = Page::Library;
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                draw(f, &mut state, f.area());
            })
            .unwrap();
    }

    #[test]
    fn test_draw_settings_does_not_panic() {
        let mut state = test_state();
        state.current_page = Page::Settings;
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                draw(f, &mut state, f.area());
            })
            .unwrap();
    }

    #[test]
    fn test_draw_reader_does_not_panic() {
        let mut state = test_state();
        state.library.add_book("Test Book".into(), "url".into());
        state.current_page = Page::Reader;
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                draw(f, &mut state, f.area());
            })
            .unwrap();
    }
}
