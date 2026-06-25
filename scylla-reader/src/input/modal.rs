use crate::messenger::AppCommand;
use crate::state::{AppState, Modal, Page};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub fn handle_adding_book(
    state: &mut AppState,
    key: KeyEvent,
    cmd_tx: &std::sync::mpsc::Sender<AppCommand>,
) -> bool {
    match (key.modifiers, key.code) {
        (KeyModifiers::CONTROL, KeyCode::Char('s')) => {
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
                crate::settings::log_debug("No valid URLs to scrape");
            } else {
                crate::settings::log_debug(&format!("Submitting {} URLs", urls.len()));
                for url in urls {
                    if let Err(e) = cmd_tx.send(AppCommand::Scrape(url)) {
                        eprintln!("Failed to queue scrape: {}", e);
                    }
                }
            }

            state.modal = Modal::None;
            state.current_page = Page::Library;
            return true;
        }
        (_, KeyCode::Esc) => {
            state.modal = Modal::None;
            state.current_page = Page::Library;
            return true;
        }
        _ => {}
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
            KeyCode::Esc => {
                state.modal = Modal::None;
                state.current_page = Page::Library;
                return true;
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
            book.progress.current = cursor_val as u32;
            if let Err(err) =
                state
                    .db
                    .update_progress(&book.url, book.progress.current, book.progress.total)
            {
                crate::settings::log_debug(&format!("Failed to update book progress. e: {}", err));
                return false;
            }
        }
        state.modal = Modal::None;
        state.current_page = Page::Library;
    }

    true
}
