pub mod app;
pub mod input;
pub mod library;
pub mod models;
pub mod ui;
pub mod scrapers;
pub mod messenger;
pub mod worker;
pub mod settings;
pub mod db;

use crossterm::{
    event::{self, Event},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use ratatui::prelude::*;
use ratatui_image::picker::Picker;
use ratatui_image::protocol::StatefulProtocol;
use std::io::stdout;
use std::sync::mpsc;
use crate::models::Book;
use crate::messenger::{AppCommand, ChapterContent};
use crate::db::Db;

fn cleanup() {
    let _ = stdout().execute(LeaveAlternateScreen);
    let _ = disable_raw_mode();
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    std::panic::set_hook(Box::new(|info| {
        cleanup();
        eprintln!("Application panicked: {}", info);
    }));

    let (cmd_tx, cmd_rx) = mpsc::channel::<AppCommand>();
    let (ui_tx, ui_rx) = mpsc::channel::<Book>();
    let (chapter_tx, chapter_rx) = mpsc::channel::<ChapterContent>();
    let (cover_tx, cover_rx) = mpsc::channel::<(String, StatefulProtocol)>();

    std::thread::spawn(move || {
        let worker = worker::Worker::new(cmd_rx, ui_tx, chapter_tx);
        worker.run();
    });

    // Must happen before EnterAlternateScreen
    let picker = Picker::from_query_stdio()
        .unwrap_or_else(|_| Picker::from_fontsize((8, 12)));
    let picker_font_size = picker.font_size();
    let picker_protocol_type = picker.protocol_type();

    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
    let mut state = app::AppState::new();

    // Load persisted library
    let db = Db::open().unwrap_or_else(|e| {
        crate::settings::log_debug(&format!("DB open failed: {}", e));
        panic!("Could not open database");
    });
    for book in db.load_books().unwrap_or_default() {
        state.library.books.push(book);
    }

    let mut last_cover_url: Option<String> = None;

    loop {
        // Handle incoming books from scraper
        if let Ok(book) = ui_rx.try_recv() {
            crate::settings::log_debug(&format!("UI received book: {}", book.title));
            if let Some(existing) = state.library.books.iter_mut().find(|b| b.url == book.url) {
                existing.title       = book.title.clone();
                existing.progress.total = book.progress.total;
                existing.cover_url   = book.cover_url.clone();
                existing.description = book.description.clone();
                existing.chapters    = book.chapters.clone();
            } else {
                state.library.books.push(book.clone());
            }
            // Persist the upserted book
            if let Some(b) = state.library.books.iter().find(|b| b.url == book.url) {
                db.upsert_book(b).unwrap_or_else(|e| {
                    crate::settings::log_debug(&format!("DB upsert failed: {}", e));
                });
            }
            last_cover_url = None;
            state.library.cached_cover = None;
            state.library.cached_cover_url = None;
        }

        // Handle incoming chapter content
        if let Ok(chapter) = chapter_rx.try_recv() {
            crate::settings::log_debug(&format!("Chapter received: {}", chapter.title));
            if let Some(book) = state.library.selected_book_mut() {
                book.progress.current = chapter.chapter_idx as u32;
            }
            // Persist updated progress
            if let Some(book) = state.library.selected_book() {
                db.update_progress(&book.url, book.progress.current, book.progress.total)
                    .unwrap_or_else(|e| {
                        crate::settings::log_debug(&format!("DB progress update failed: {}", e));
                    });
            }
            state.open_reader_chapter(
                chapter.title,
                chapter.content,
                chapter.chapter_idx,
            );
        }

        // Receive finished cover protocol from background thread
        if let Ok((url, protocol)) = cover_rx.try_recv() {
            let current_url = state.library
                .selected_book()
                .and_then(|b| b.cover_url.as_deref().map(str::to_owned));
            if current_url.as_deref() == Some(&url) {
                state.library.cached_protocol = Some(protocol);
            }
        }

        // Kick off background cover load if selected book changed
        let current_cover_url = state.library
            .selected_book()
            .and_then(|b| b.cover_url.clone());

        if current_cover_url != last_cover_url {
            last_cover_url = current_cover_url.clone();
            state.library.cached_cover = None;
            state.library.cached_cover_url = None;
            state.library.cached_protocol = None;

            if let Some(url) = current_cover_url {
                let cover_tx = cover_tx.clone();
                std::thread::spawn(move || {
                    let mut picker = Picker::from_fontsize(picker_font_size);
                    picker.set_protocol_type(picker_protocol_type);
                    match reqwest::blocking::get(&url) {
                        Ok(resp) => match resp.bytes() {
                            Ok(bytes) => match image::load_from_memory(&bytes) {
                                Ok(img) => {
                                    let protocol = picker.new_resize_protocol(img);
                                    let _ = cover_tx.send((url, protocol));
                                }
                                Err(e) => crate::settings::log_debug(&format!("Image decode: {}", e)),
                            },
                            Err(e) => crate::settings::log_debug(&format!("Cover bytes: {}", e)),
                        },
                        Err(e) => crate::settings::log_debug(&format!("Cover fetch: {}", e)),
                    }
                });
            }
        }

        let mut last_area = Rect::default();
        terminal.draw(|f| {
            last_area = f.area();
            ui::draw(f, &mut state, last_area);
        })?;

        if event::poll(std::time::Duration::from_millis(16))? {
            if let Event::Key(key) = event::read()? {
                // Capture pre-input state for DB sync
                let pre_status = state.library.selected_book()
                    .map(|b| (b.url.clone(), b.status.clone()));
                let pre_books_len = state.library.books.len();
                let removed_url = if key.code == crossterm::event::KeyCode::Char('d') {
                    state.library.selected_book().map(|b| b.url.clone())
                } else {
                    None
                };

                if !input::handle_input(&mut state, key, &cmd_tx, last_area) {
                    break;
                }

                // Persist delete
                if let Some(url) = removed_url {
                    if state.library.books.len() < pre_books_len {
                        db.delete_book(&url).unwrap_or_else(|e| {
                            crate::settings::log_debug(&format!("DB delete failed: {}", e));
                        });
                    }
                }

                // Persist status change
                if let Some((url, old_status)) = pre_status {
                    if let Some(book) = state.library.books.iter().find(|b| b.url == url) {
                        if book.status != old_status {
                            db.update_status(&book.url, &book.status).unwrap_or_else(|e| {
                                crate::settings::log_debug(&format!("DB status update failed: {}", e));
                            });
                        }
                    }
                }
            }
        }
    }

    cleanup();
    Ok(())
}
