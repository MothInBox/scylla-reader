//! Render dispatch — routes by current page to page-specific draw functions.

pub mod jobs;
pub mod library;
pub mod modal;
pub mod palette;
pub mod reader;
pub mod settings;
pub mod widgets;

use crate::state::AppState;
use crate::state::page::Page;
use ratatui::prelude::*;

/// Truncate a snippet to `width` visible characters, appending an ellipsis when
/// it was cut. Multi-byte safe (counts chars, not bytes).
pub(crate) fn truncate_snippet(s: &str, width: usize) -> String {
    if width == 0 || s.chars().count() <= width {
        return s.to_string();
    }
    let mut out: String = s.chars().take(width.saturating_sub(1)).collect();
    out.push('…');
    out
}

pub fn draw(frame: &mut Frame, state: &mut AppState, area: Rect) {
    match state.ui.page {
        Page::Library => {
            library::draw(frame, area, &mut state.lib, &state.ui);
        }
        Page::Settings => {
            settings::draw(frame, area, &state.lib, &state.ui);
        }
        Page::Reader => {
            reader::draw(frame, area, &state.reader, &state.lib, &state.ui);
        }
        Page::Jobs => {
            jobs::draw(
                frame,
                area,
                &mut state.jobs,
                &state.ui,
                state.lib.settings.rate_limit_secs,
            );
        }
    }

    modal::draw_modal(frame, area, &mut state.ui, &mut state.lib);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Page;
    use crate::test_helpers::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    #[test]
    fn test_draw_library_does_not_panic() {
        let mut state = test_state();
        state.ui.page = Page::Library;
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
        state.ui.page = Page::Settings;
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
        state.lib.library.add_book("Test Book".into(), "url".into());
        state.ui.page = Page::Reader;
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                draw(f, &mut state, f.area());
            })
            .unwrap();
    }
}
