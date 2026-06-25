use crate::messenger::AppCommand;
use crate::state::{AppState, Page};
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::prelude::Rect;

pub fn handle_reader(
    state: &mut AppState,
    key: KeyEvent,
    cmd_tx: &std::sync::mpsc::Sender<AppCommand>,
    size: Rect,
) -> bool {
    use crate::settings::ReaderMode;

    match (key.modifiers, key.code) {
        (_, KeyCode::Char(c)) if c == '>' => {
            if !state.reader.loading {
                if let Some(book) = state.library.selected_book() {
                    let next_idx = state.reader.current_chapter_idx + 1;
                    if let Some(ch) = book.chapters.get(next_idx) {
                        let url = ch.url.clone();
                        state.reader.loading = true;
                        let _ = cmd_tx.send(AppCommand::FetchChapter(url, next_idx));
                    }
                }
            }
            return true;
        }
        (_, KeyCode::Char(c)) if c == '<' => {
            if !state.reader.loading {
                if let Some(book) = state.library.selected_book() {
                    let prev_idx = state.reader.current_chapter_idx.saturating_sub(1);
                    if prev_idx != state.reader.current_chapter_idx {
                        if let Some(ch) = book.chapters.get(prev_idx) {
                            let url = ch.url.clone();
                            state.reader.loading = true;
                            let _ = cmd_tx.send(AppCommand::FetchChapter(url, prev_idx));
                        }
                    }
                }
            }
            return true;
        }
        (_, KeyCode::Esc) => {
            state.current_page = Page::Library;
            return true;
        }
        _ => {}
    }

    match state.settings.reader_mode {
        ReaderMode::Paged => match key.code {
            KeyCode::Right | KeyCode::Char('l') => {
                state
                    .reader
                    .next_page(size.width, size.height.saturating_sub(2));
                true
            }
            KeyCode::Left | KeyCode::Char('h') => {
                state
                    .reader
                    .prev_page(size.width, size.height.saturating_sub(2));
                true
            }
            _ => true,
        },
        ReaderMode::Scrollable => match key.code {
            KeyCode::Down | KeyCode::Char('j') => {
                state.reader.scroll_down(1);
                true
            }
            KeyCode::Up | KeyCode::Char('k') => {
                state.reader.scroll_up(1);
                true
            }
            KeyCode::PageDown => {
                state.reader.scroll_down(20);
                true
            }
            KeyCode::PageUp => {
                state.reader.scroll_up(20);
                true
            }
            _ => true,
        },
    }
}
