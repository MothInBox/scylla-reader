use crate::messenger::AppCommand;
use crate::state::{AppState, Modal, Page};
use crossterm::event::{KeyCode, KeyEvent};

pub fn handle_library(
    state: &mut AppState,
    key: KeyEvent,
    cmd_tx: &std::sync::mpsc::Sender<AppCommand>,
) -> bool {
    let total_books = state.library.books.len();
    match key.code {
        KeyCode::Char('q') => false,
        KeyCode::Tab => {
            state.current_page = Page::Settings;
            true
        }
        KeyCode::Char('i') => {
            state.modal = Modal::AddBook {
                inputs: vec![String::new()],
                cursor: 0,
                scroll_offset: 0,
            };
            state.current_page = Page::AddingBook;
            true
        }
        KeyCode::Char('j') => {
            if let Some(book) = state.library.selected_book() {
                crate::settings::log_debug(&format!("Adding Book {} to modal", book.title));
                state.modal = Modal::JumpChapter {
                    chapters: book.chapters.clone(),
                    cursor: 0,
                    scroll_offset: 0,
                };
                state.current_page = Page::BookChapterJump;
            }
            true
        }
        KeyCode::Char('d') => {
            state.library.remove_selected();
            true
        }
        KeyCode::Char(' ') => {
            state.library.cycle_selected_status();
            true
        }
        KeyCode::Char('f') => {
            state.library.cycle_filter();
            true
        }
        KeyCode::Enter => {
            if let Some(book) = state.library.selected_book() {
                if book.chapters.is_empty() {
                    let book_title = book.title.clone();
                    let desc = book
                        .description
                        .clone()
                        .unwrap_or_else(|| "No content available.".to_string());
                    crate::settings::log_debug(&format!("No chapters for: {}", book_title));
                    state.open_reader_chapter("Description".to_string(), desc, 0);
                } else {
                    let idx = (book.progress.current as usize).min(book.chapters.len() - 1);
                    let chapter_url = book.chapters[idx].url.clone();
                    state.reader.loading = true;
                    state.current_page = Page::Reader;
                    if let Err(e) = cmd_tx.send(AppCommand::FetchChapter(chapter_url, idx)) {
                        crate::settings::log_debug(&format!("Failed to queue chapter: {}", e));
                    }
                }
            }
            true
        }
        KeyCode::Char('u') => {
            let urls: Vec<String> = state
                .library
                .books
                .iter()
                .map(|b| b.url.clone())
                .filter(|u| !u.is_empty())
                .collect();
            if !urls.is_empty() {
                crate::settings::log_debug("Attempting to update all books...");
                if let Err(e) = cmd_tx.send(AppCommand::UpdateAll(urls)) {
                    eprintln!("Failed to queue update: {}", e);
                }
            }
            true
        }
        KeyCode::Down => {
            if total_books > 0 {
                state.library.selected_index =
                    (state.library.selected_index + 1).min(total_books - 1);
            }
            true
        }
        KeyCode::Up => {
            if total_books > 0 {
                state.library.selected_index = state.library.selected_index.saturating_sub(1);
            }
            true
        }
        _ => true,
    }
}
