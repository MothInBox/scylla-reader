//! Owns the terminal, channels, state, and main loop. Bridges worker events,
//! input dispatch, cover loading, and UI rendering.

use crate::input;
use crate::messenger::{AppCommand, AppEvent};
use crate::scrapers::services::ScraperRegistry;
use crate::settings::SettingsPage;
use crate::state::{AppState, Modal, Page};
use crate::ui;
use crate::worker;

use crossterm::{
    ExecutableCommand,
    event::{self, Event, KeyCode},
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::prelude::*;
use std::io::stdout;
use std::sync::mpsc;
use std::time::Duration;

pub struct App {
    terminal: Terminal<CrosstermBackend<Box<dyn std::io::Write>>>,
    state: AppState,
    cmd_tx: mpsc::Sender<AppCommand>,
    event_rx: mpsc::Receiver<AppEvent>,
    last_cover_url: Option<String>,
}

impl App {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let (cmd_tx, cmd_rx) = mpsc::channel::<AppCommand>();
        let (event_tx, event_rx) = mpsc::channel::<AppEvent>();

        let registry = ScraperRegistry::new();
        let worker_event_tx = event_tx;
        std::thread::spawn(move || {
            let worker = worker::Worker::new(cmd_rx, worker_event_tx, registry);
            worker.run();
        });

        let terminal = Terminal::new(CrosstermBackend::new(
            Box::new(stdout()) as Box<dyn std::io::Write>
        ))?;

        let mut state = AppState::new();
        for book in state.db.load_books().unwrap_or_default() {
            state.library.books.push(book);
        }

        Ok(Self {
            terminal,
            state,
            cmd_tx,
            event_rx,
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

            if event::poll(Duration::from_millis(16))?
                && let Event::Key(key) = event::read()?
                && !self.handle_key(key, area)
            {
                break Ok(());
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
                    crate::settings::log(
                        crate::settings::LogLevel::Debug,
                        "UI",
                        &format!("UI received book: {}", book.title),
                    );
                    if let Some(existing) = self
                        .state
                        .library
                        .books
                        .iter_mut()
                        .find(|b| b.url == book.url)
                    {
                        existing.title = book.title.clone();
                        existing.cover_url = book.cover_url.clone();
                        existing.description = book.description.clone();
                        existing.chapters = book.chapters.clone();
                        self.state.db.upsert_book(existing).unwrap_or_else(|e| {
                            crate::settings::log(
                                crate::settings::LogLevel::Debug,
                                "UI",
                                &format!("DB upsert failed: {}", e),
                            );
                        });
                        if let Ok(sessions) = self.state.db.load_sessions_for_book(&existing.url) {
                            existing.sessions = sessions;
                        }
                    } else {
                        self.state.db.upsert_book(&book).unwrap_or_else(|e| {
                            crate::settings::log(
                                crate::settings::LogLevel::Debug,
                                "UI",
                                &format!("DB upsert failed: {}", e),
                            );
                        });
                        let book_url = book.url.clone();
                        self.state.library.books.push(book);
                        if let Some(b) = self
                            .state
                            .library
                            .books
                            .iter_mut()
                            .find(|b| b.url == book_url)
                        {
                            if let Ok(sessions) = self.state.db.load_sessions_for_book(&book_url) {
                                b.sessions = sessions;
                            }
                        }
                    }
                    self.last_cover_url = None;
                }
                AppEvent::ChapterFetched(chapter) => {
                    crate::settings::log(
                        crate::settings::LogLevel::Debug,
                        "UI",
                        &format!("Chapter received: {}", chapter.title),
                    );
                    if let Some(book) = self.state.library.selected_book_mut() {
                        if let Some(session) = book
                            .sessions
                            .iter_mut()
                            .find(|s| s.id == self.state.reader.session_id)
                        {
                            session.progress.current = chapter.chapter_idx as u32;
                            self.state
                                .db
                                .update_session_progress(session.id, session.progress.current)
                                .unwrap_or_else(|e| {
                                    crate::settings::log(
                                        crate::settings::LogLevel::Debug,
                                        "UI",
                                        &format!("DB session progress update failed: {}", e),
                                    );
                                });
                            self.state
                                .db
                                .set_active_session(&book.url, Some(session.id))
                                .unwrap_or_else(|e| {
                                    crate::settings::log(
                                        crate::settings::LogLevel::Debug,
                                        "UI",
                                        &format!("DB set active session failed: {}", e),
                                    );
                                });
                        }
                    }
                    self.state.open_reader_chapter(
                        chapter.title,
                        chapter.content,
                        chapter.chapter_idx,
                        self.state.reader.session_id,
                        self.state.reader.session_name.clone(),
                    );
                }
                AppEvent::CoverFetched(url, protocol) => {
                    let current_url = self
                        .state
                        .library
                        .selected_book()
                        .and_then(|b| b.cover_url.as_deref().map(str::to_owned));
                    if current_url.as_deref() == Some(&url) {
                        self.state.library.cached_protocol = Some(protocol);
                    }
                }
            }
        }
    }

    fn update_covers(&mut self) {
        let current_cover_url = self
            .state
            .library
            .selected_book()
            .and_then(|b| b.cover_url.clone());

        if current_cover_url != self.last_cover_url {
            self.last_cover_url = current_cover_url.clone();
            self.state.library.cached_protocol = None;

            if let Some(url) = current_cover_url {
                let _ = self.cmd_tx.send(AppCommand::FetchCover(url));
            }
        }
    }

    #[cfg(test)]
    pub fn test_instance(state: AppState) -> Self {
        let backend = CrosstermBackend::new(Box::new(std::io::sink()) as Box<dyn std::io::Write>);
        let terminal = Terminal::new(backend).unwrap();
        let (cmd_tx, _cmd_rx) = mpsc::channel::<AppCommand>();
        let (_event_tx, event_rx) = mpsc::channel::<AppEvent>();
        Self {
            terminal,
            state,
            cmd_tx,
            event_rx,
            last_cover_url: None,
        }
    }

    fn handle_key(&mut self, key: crossterm::event::KeyEvent, area: Rect) -> bool {
        if self.state.modal != Modal::None {
            if key.code == KeyCode::Esc {
                self.state.close_modal();
                if matches!(
                    self.state.current_page,
                    Page::AddingBook | Page::BookChapterJump
                ) {
                    self.state.current_page = Page::Library;
                }
                return true;
            }
            return input::handle_input(&mut self.state, key, &self.cmd_tx, area);
        }

        match key.code {
            KeyCode::Char('1') => {
                self.state.current_page = Page::Library;
                return true;
            }
            KeyCode::Char('2') => {
                self.state.current_page = Page::Reader;
                if let Some(book) = self.state.library.selected_book()
                    && self.state.reader.book_url != book.url
                {
                    let session = book
                        .active_session_id
                        .and_then(|id| book.sessions.iter().find(|s| s.id == id))
                        .or_else(|| book.sessions.first());
                    if let Some(session) = session {
                        self.state.reader.session_id = session.id;
                        self.state.reader.session_name = session.name.clone();
                        let idx = (session.progress.current as usize)
                            .min(book.chapters.len().saturating_sub(1));
                        if let Some(ch) = book.chapters.get(idx) {
                            self.state.reader.loading = true;
                            let _ = self
                                .cmd_tx
                                .send(AppCommand::FetchChapter(ch.url.clone(), idx));
                        }
                    }
                }
                return true;
            }
            KeyCode::Char('3') => {
                self.state.current_page = Page::Settings;
                return true;
            }
            KeyCode::Char(':') => {
                let actions = crate::ui::palette::build_palette_actions(self.cmd_tx.clone());
                let filtered = crate::ui::palette::filter_actions(&actions, "");
                self.state.modal = Modal::CommandPalette {
                    query: String::new(),
                    filtered,
                    selected: 0,
                };
                return true;
            }
            KeyCode::Char('?') => {
                self.state.show_hints = !self.state.show_hints;
                return true;
            }

            KeyCode::Esc => {
                if self.state.current_page == Page::Settings {
                    match self.state.settings.settings_page {
                        SettingsPage::Main => return false,
                        SettingsPage::DebugLog => {
                            self.state.settings.settings_page = SettingsPage::Main;
                        }
                        SettingsPage::PluginList => {
                            self.state.settings.settings_page = SettingsPage::Main;
                        }
                        SettingsPage::PluginFields => {
                            self.state.settings.settings_page = SettingsPage::PluginList;
                        }
                        SettingsPage::PluginFieldEdit => {
                            self.state.settings.plugin_field_buffer.clear();
                            self.state.settings.plugin_field_editing = false;
                            self.state.settings.settings_page = SettingsPage::PluginFields;
                        }
                    }
                    return true;
                }
                return false;
            }
            _ => {}
        }

        let pre_status = self
            .state
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

        if let Some(url) = removed_url
            && self.state.library.books.len() < pre_books_len
        {
            self.state.db.delete_book(&url).unwrap_or_else(|e| {
                crate::settings::log(
                    crate::settings::LogLevel::Debug,
                    "UI",
                    &format!("DB delete failed: {}", e),
                );
            });
        }

        if let Some((url, old_status)) = pre_status
            && let Some(book) = self.state.library.books.iter().find(|b| b.url == url)
            && book.status != old_status
        {
            self.state
                .db
                .update_status(&book.url, &book.status)
                .unwrap_or_else(|e| {
                    crate::settings::log(
                        crate::settings::LogLevel::Debug,
                        "UI",
                        &format!("DB status update failed: {}", e),
                    );
                });
        }

        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;
    use crate::library::Library;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn test_state() -> AppState {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let db = Db::open_conn(conn).unwrap();
        AppState::from_parts(db, Library::new())
    }

    fn key_event(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn test_global_key_1_switches_to_library() {
        let mut app = App::test_instance(test_state());
        app.state.current_page = Page::Reader;
        app.handle_key(key_event(KeyCode::Char('1')), Rect::default());
        assert_eq!(app.state.current_page, Page::Library);
    }

    #[test]
    fn test_global_key_2_switches_to_reader() {
        let mut app = App::test_instance(test_state());
        app.state.current_page = Page::Library;
        app.handle_key(key_event(KeyCode::Char('2')), Rect::default());
        assert_eq!(app.state.current_page, Page::Reader);
    }

    #[test]
    fn test_global_key_3_switches_to_settings() {
        let mut app = App::test_instance(test_state());
        app.state.current_page = Page::Library;
        app.handle_key(key_event(KeyCode::Char('3')), Rect::default());
        assert_eq!(app.state.current_page, Page::Settings);
    }

    #[test]
    fn test_colon_opens_command_palette() {
        let mut app = App::test_instance(test_state());
        assert_eq!(app.state.modal, Modal::None);
        app.handle_key(key_event(KeyCode::Char(':')), Rect::default());
        assert!(matches!(app.state.modal, Modal::CommandPalette { .. }));
    }

    #[test]
    fn test_question_toggles_hints() {
        let mut app = App::test_instance(test_state());
        let initial = app.state.show_hints;
        app.handle_key(key_event(KeyCode::Char('?')), Rect::default());
        assert_eq!(app.state.show_hints, !initial);
        app.handle_key(key_event(KeyCode::Char('?')), Rect::default());
        assert_eq!(app.state.show_hints, initial);
    }

    #[test]
    fn test_esc_closes_modal() {
        let mut app = App::test_instance(test_state());
        app.state.modal = Modal::CommandPalette {
            query: "".into(),
            filtered: vec![],
            selected: 0,
        };
        app.state.current_page = Page::AddingBook;
        let result = app.handle_key(key_event(KeyCode::Esc), Rect::default());
        assert!(result);
        assert_eq!(app.state.modal, Modal::None);
    }

    #[test]
    fn test_esc_quits_from_library() {
        let mut app = App::test_instance(test_state());
        app.state.current_page = Page::Library;
        let result = app.handle_key(key_event(KeyCode::Esc), Rect::default());
        assert!(!result);
    }

    #[test]
    fn test_esc_quits_from_any_page() {
        let mut app = App::test_instance(test_state());
        app.state.current_page = Page::Settings;
        let result = app.handle_key(key_event(KeyCode::Esc), Rect::default());
        assert!(!result);
    }

    #[test]
    fn test_colon_does_not_open_palette_when_modal_open() {
        let mut app = App::test_instance(test_state());
        app.state.modal = Modal::AddBook {
            inputs: vec![String::new()],
            cursor: 0,
            scroll_offset: 0,
        };
        app.handle_key(key_event(KeyCode::Char(':')), Rect::default());
        assert!(matches!(app.state.modal, Modal::AddBook { .. }));
    }

    #[test]
    fn test_global_key_does_not_switch_page_when_modal_open() {
        let mut app = App::test_instance(test_state());
        app.state.current_page = Page::Library;
        app.state.modal = Modal::AddBook {
            inputs: vec![String::new()],
            cursor: 0,
            scroll_offset: 0,
        };
        app.handle_key(key_event(KeyCode::Char('2')), Rect::default());
        assert_eq!(app.state.current_page, Page::Library);
    }
}
