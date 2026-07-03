//! Owns the terminal, channels, state, and main loop. Bridges worker events,
//! input dispatch, cover loading, and UI rendering.

use crate::input;
use crate::messenger::{AppCommand, AppEvent};
use crate::scrapers::services::ScraperRegistry;
use crate::state::AppState;
use crate::ui;
use crate::worker;

use crossterm::{
    ExecutableCommand,
    event::{self, Event, KeyCode},
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::prelude::*;
use ratatui_image::picker::Picker;
use ratatui_image::protocol::StatefulProtocol;
use std::io::stdout;
use std::sync::mpsc;
use std::time::Duration;

pub struct App {
    terminal: Terminal<CrosstermBackend<std::io::Stdout>>,
    state: AppState,
    cmd_tx: mpsc::Sender<AppCommand>,
    event_rx: mpsc::Receiver<AppEvent>,
    cover_tx: mpsc::Sender<(String, StatefulProtocol)>,
    cover_rx: mpsc::Receiver<(String, StatefulProtocol)>,
    picker_font_size: (u16, u16),
    picker_protocol_type: ratatui_image::picker::ProtocolType,
    last_cover_url: Option<String>,
}

impl App {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let (cmd_tx, cmd_rx) = mpsc::channel::<AppCommand>();
        let (event_tx, event_rx) = mpsc::channel::<AppEvent>();
        let (cover_tx, cover_rx) = mpsc::channel::<(String, StatefulProtocol)>();

        let registry = ScraperRegistry::new();
        let worker_event_tx = event_tx;
        std::thread::spawn(move || {
            let worker = worker::Worker::new(cmd_rx, worker_event_tx, registry);
            worker.run();
        });

        let picker = Picker::from_query_stdio().unwrap_or_else(|_| Picker::from_fontsize((8, 12)));
        #[cfg(feature = "vhs-mode")]
        let picker = {
            let mut p = picker;
            p.set_protocol_type(ratatui_image::picker::ProtocolType::Halfblocks);
            p
        };
        let picker_font_size = picker.font_size();
        let picker_protocol_type = picker.protocol_type();

        let terminal = Terminal::new(CrosstermBackend::new(stdout()))?;

        let mut state = AppState::new();
        for book in state.db.load_books().unwrap_or_default() {
            state.library.books.push(book);
        }

        Ok(Self {
            terminal,
            state,
            cmd_tx,
            event_rx,
            cover_tx,
            cover_rx,
            picker_font_size,
            picker_protocol_type,
            last_cover_url: None,
        })
    }

    pub fn run(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        enable_raw_mode()?;
        stdout().execute(EnterAlternateScreen)?;

        let result = self.main_loop();

        let _ = stdout().execute(LeaveAlternateScreen);
        let _ = disable_raw_mode();

        result
    }

    fn main_loop(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        loop {
            self.drain_events();
            self.update_covers();

            let area = self.draw()?;

            if event::poll(Duration::from_millis(16))? {
                if let Event::Key(key) = event::read()? {
                    if !self.handle_key(key, area) {
                        break Ok(());
                    }
                }
            }
        }
    }

    fn draw(&mut self) -> Result<Rect, Box<dyn std::error::Error>> {
        let mut area = Rect::default();
        self.terminal.draw(|f| {
            area = f.area();
            ui::draw(f, &mut self.state, area);
        })?;
        Ok(area)
    }

    fn drain_events(&mut self) {
        while let Ok(event) = self.event_rx.try_recv() {
            match event {
                AppEvent::BookScraped(book) => {
                    crate::settings::log(crate::settings::LogLevel::Debug, "UI", &format!("UI received book: {}", book.title));
                    if let Some(existing) = self.state.library.books.iter_mut().find(|b| b.url == book.url) {
                        existing.title = book.title.clone();
                        existing.progress.total = book.progress.total;
                        existing.cover_url = book.cover_url.clone();
                        existing.description = book.description.clone();
                        existing.chapters = book.chapters.clone();
                    } else {
                        self.state.library.books.push(book.clone());
                    }
                    if let Some(b) = self.state.library.books.iter().find(|b| b.url == book.url) {
                        self.state.db.upsert_book(b).unwrap_or_else(|e| {
                            crate::settings::log(crate::settings::LogLevel::Debug, "UI", &format!("DB upsert failed: {}", e));
                        });
                    }
                    self.last_cover_url = None;
                    self.state.library.cached_cover = None;
                    self.state.library.cached_cover_url = None;
                }
                AppEvent::ChapterFetched(chapter) => {
                    crate::settings::log(crate::settings::LogLevel::Debug, "UI", &format!("Chapter received: {}", chapter.title));
                    if let Some(book) = self.state.library.selected_book_mut() {
                        book.progress.current = chapter.chapter_idx as u32;
                    }
                    if let Some(book) = self.state.library.selected_book() {
                        self.state
                            .db
                            .update_progress(&book.url, book.progress.current, book.progress.total)
                            .unwrap_or_else(|e| {
                                crate::settings::log(crate::settings::LogLevel::Debug, "UI", &format!("DB progress update failed: {}", e));
                            });
                    }
                    self.state.open_reader_chapter(chapter.title, chapter.content, chapter.chapter_idx);
                }
            }
        }

        while let Ok((url, protocol)) = self.cover_rx.try_recv() {
            let current_url = self.state
                .library
                .selected_book()
                .and_then(|b| b.cover_url.as_deref().map(str::to_owned));
            if current_url.as_deref() == Some(&url) {
                self.state.library.cached_protocol = Some(protocol);
            }
        }
    }

    fn update_covers(&mut self) {
        let current_cover_url = self.state
            .library
            .selected_book()
            .and_then(|b| b.cover_url.clone());

        if current_cover_url != self.last_cover_url {
            self.last_cover_url = current_cover_url.clone();
            self.state.library.cached_cover = None;
            self.state.library.cached_cover_url = None;
            self.state.library.cached_protocol = None;

            if let Some(url) = current_cover_url {
                let cover_tx = self.cover_tx.clone();
                let picker_font_size = self.picker_font_size;
                let picker_protocol_type = self.picker_protocol_type;
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
                                Err(e) => crate::settings::log(crate::settings::LogLevel::Debug, "UI", &format!("Image decode: {}", e)),
                            },
                            Err(e) => crate::settings::log(crate::settings::LogLevel::Debug, "UI", &format!("Cover bytes: {}", e)),
                        },
                        Err(e) => crate::settings::log(crate::settings::LogLevel::Debug, "UI", &format!("Cover fetch: {}", e)),
                    }
                });
            }
        }
    }

    fn handle_key(&mut self, key: crossterm::event::KeyEvent, area: Rect) -> bool {
        let pre_status = self.state
            .library
            .selected_book()
            .map(|b| (b.url.clone(), b.status.clone()));
        let pre_books_len = self.state.library.books.len();
        let removed_url = if key.code == KeyCode::Char('d') {
            self.state.library.selected_book().map(|b| b.url.clone())
        } else {
            None
        };

        if !input::handle_input(&mut self.state, key, &self.cmd_tx, area) {
            return false;
        }

        if let Some(url) = removed_url {
            if self.state.library.books.len() < pre_books_len {
                self.state.db.delete_book(&url).unwrap_or_else(|e| {
                    crate::settings::log(crate::settings::LogLevel::Debug, "UI", &format!("DB delete failed: {}", e));
                });
            }
        }

        if let Some((url, old_status)) = pre_status {
            if let Some(book) = self.state.library.books.iter().find(|b| b.url == url) {
                if book.status != old_status {
                    self.state
                        .db
                        .update_status(&book.url, &book.status)
                        .unwrap_or_else(|e| {
                            crate::settings::log(crate::settings::LogLevel::Debug, "UI", &format!("DB status update failed: {}", e));
                        });
                }
            }
        }

        true
    }
}
