use crate::db::Db;
use crate::messenger::AppCommand;
use crate::settings::{SettingsField, SettingsPage};
use crate::state::{AppState, Modal, Page};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::prelude::Rect;

pub fn handle_input(
    state: &mut AppState,
    key: KeyEvent,
    cmd_tx: &std::sync::mpsc::Sender<AppCommand>,
    size: Rect,
) -> bool {
    match state.current_page.clone() {
        Page::AddingBook => handle_adding_book(state, key, cmd_tx),
        Page::Library => handle_library(state, key, cmd_tx),
        Page::Settings => handle_settings(state, key),
        Page::Reader => handle_reader(state, key, cmd_tx, size),
        Page::BookChapterJump => handle_jumping_chapter(state, key),
    }
}

fn handle_adding_book(
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

fn handle_library(
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

fn handle_settings(state: &mut AppState, key: KeyEvent) -> bool {
    match state.settings.settings_page.clone() {
        SettingsPage::Main => handle_settings_main(state, key),
        SettingsPage::CookieList => handle_cookie_list(state, key),
        SettingsPage::CookieEdit => handle_cookie_edit(state, key),
    }
}

fn handle_settings_main(state: &mut AppState, key: KeyEvent) -> bool {
    let num_fields = SettingsField::all().len();
    match key.code {
        KeyCode::Tab | KeyCode::Esc => {
            state.current_page = Page::Library;
            true
        }
        KeyCode::Down => {
            state.settings.selected_field = (state.settings.selected_field + 1).min(num_fields - 1);
            true
        }
        KeyCode::Up => {
            state.settings.selected_field = state.settings.selected_field.saturating_sub(1);
            true
        }
        KeyCode::Enter => {
            match SettingsField::all()[state.settings.selected_field] {
                SettingsField::Cookies => {
                    state.settings.reload_cookies();
                    state.settings.settings_page = SettingsPage::CookieList;
                }
                SettingsField::RateLimit => {
                    state.settings.edit_buffer = state.settings.rate_limit_secs.to_string();
                    state.settings.editing = true;
                }
                SettingsField::DebugLog => {
                    state.settings.debug_log = !state.settings.debug_log;
                    crate::settings::set_debug(state.settings.debug_log);
                    if state.settings.debug_log {
                        let _ = std::fs::write(crate::settings::LOG_FILE, "");
                        crate::settings::log_debug("Debug logging enabled");
                    }
                }
                SettingsField::ReaderMode => {
                    state.settings.reader_mode = state.settings.reader_mode.toggle();
                }
            }
            true
        }
        KeyCode::Char(c) if state.settings.editing => {
            state.settings.edit_buffer.push(c);
            true
        }
        KeyCode::Backspace if state.settings.editing => {
            state.settings.edit_buffer.pop();
            true
        }
        _ => true,
    }
}

fn handle_cookie_list(state: &mut AppState, key: KeyEvent) -> bool {
    let num_cookies = state.settings.cookie_stores.len();
    match key.code {
        KeyCode::Esc => {
            state.settings.settings_page = SettingsPage::Main;
            true
        }
        KeyCode::Down => {
            if num_cookies > 0 {
                state.settings.selected_cookie =
                    (state.settings.selected_cookie + 1).min(num_cookies - 1);
            }
            true
        }
        KeyCode::Up => {
            state.settings.selected_cookie = state.settings.selected_cookie.saturating_sub(1);
            true
        }
        KeyCode::Enter => {
            if let Some(store) = state
                .settings
                .cookie_stores
                .get(state.settings.selected_cookie)
            {
                state.settings.cookie_edit_buffer = store.load_raw();
                state.settings.settings_page = SettingsPage::CookieEdit;
            }
            true
        }
        _ => true,
    }
}

fn handle_cookie_edit(state: &mut AppState, key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Esc => {
            state.settings.cookie_edit_buffer.clear();
            state.settings.settings_page = SettingsPage::CookieList;
            true
        }
        KeyCode::Enter => {
            state.settings.save_current_cookie();
            state.settings.cookie_edit_buffer.clear();
            state.settings.settings_page = SettingsPage::CookieList;
            true
        }
        KeyCode::Char(c) => {
            state.settings.cookie_edit_buffer.push(c);
            true
        }
        KeyCode::Backspace => {
            state.settings.cookie_edit_buffer.pop();
            true
        }
        _ => true,
    }
}

fn handle_reader(
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

fn handle_jumping_chapter(state: &mut AppState, key: KeyEvent) -> bool {
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
            _ => {}
        }
    }

    if let Some(cursor_val) = selected_cursor {
        if let Some(book) = state.library.selected_book_mut() {
            let db = Db::open().unwrap_or_else(|e| {
                crate::settings::log_debug(&format!("DB open failed {}", e));
                panic!("Could not open database for jump update")
            });
            book.progress.current = cursor_val as u32;
            db.update_progress(&book.url, book.progress.current, book.progress.total);
        }
        state.modal = Modal::None;
        state.current_page = Page::Library;
    }

    true
}
