//! Owns the terminal, channels, state, and main loop. Bridges worker events,
//! input dispatch, cover loading, and UI rendering.

use crate::event;
use crate::key_handler;
use crate::messenger::{AppCommand, AppEvent};
use crate::scrapers::services::ScraperRegistry;
use crate::state::AppState;
use crate::ui;
use crate::worker;

use crossterm::{
    ExecutableCommand,
    event::Event,
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
    fetched_covers: std::collections::HashSet<String>,
}

impl App {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let (cmd_tx, cmd_rx) = mpsc::channel::<AppCommand>();
        let (event_tx, event_rx) = mpsc::channel::<AppEvent>();

        let picker = ratatui_image::picker::Picker::from_query_stdio()
            .unwrap_or_else(|_| ratatui_image::picker::Picker::from_fontsize((8, 12)));
        let picker_font_size = picker.font_size();
        let picker_protocol_type = picker.protocol_type();

        let registry = ScraperRegistry::new();

        let terminal = Terminal::new(CrosstermBackend::new(
            Box::new(stdout()) as Box<dyn std::io::Write>
        ))?;

        let mut state = AppState::new();
        for book in state.db.load_books().unwrap_or_default() {
            state.library.books.push(book);
        }

        let max_workers = state.settings.max_workers;
        let rate_limit = state.settings.rate_limit_secs;

        Self::spawn_worker_thread(
            cmd_rx, event_tx, registry,
            picker_font_size, picker_protocol_type,
            max_workers, rate_limit,
        );

        Ok(Self {
            terminal,
            state,
            cmd_tx,
            event_rx,
            fetched_covers: std::collections::HashSet::new(),
        })
    }

    fn spawn_worker_thread(
        cmd_rx: mpsc::Receiver<AppCommand>,
        event_tx: mpsc::Sender<AppEvent>,
        registry: ScraperRegistry,
        font_size: (u16, u16),
        protocol_type: ratatui_image::picker::ProtocolType,
        max_workers: u8,
        rate_limit: u64,
    ) {
        std::thread::Builder::new()
            .name("job-manager".to_string())
            .spawn(move || {
                let manager = worker::JobManager::new(
                    cmd_rx,
                    event_tx,
                    registry,
                    font_size,
                    protocol_type,
                    max_workers,
                    rate_limit,
                );
                manager.run();
            })
            .expect("failed to spawn job manager thread");
    }

    pub fn run(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        enable_raw_mode()?;
        stdout().execute(EnterAlternateScreen)?;
        self.terminal.clear()?;

        let result = self.main_loop();

        let _ = stdout().execute(LeaveAlternateScreen);
        let _ = disable_raw_mode();

        result
    }

    fn main_loop(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        loop {
            event::drain_events(
                &mut self.state,
                &self.event_rx,
            );
            event::update_covers(&mut self.state, &self.cmd_tx, &mut self.fetched_covers);

            let area = self.draw()?;

            if crossterm::event::poll(Duration::from_millis(16))?
                && let Event::Key(key) = crossterm::event::read()?
                && !key_handler::handle_key(&mut self.state, key, &self.cmd_tx, area)
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
            fetched_covers: std::collections::HashSet::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;
    use crate::input::keybinds::*;
    use crate::key_handler;
    use crate::library::Library;
    use crate::state::{Modal, Page};
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
        let mut state = test_state();
        state.current_page = Page::Reader;
        let (tx, _rx) = mpsc::channel();
        key_handler::handle_key(&mut state, key_event(KEY_LIBRARY), &tx, Rect::default());
        assert_eq!(state.current_page, Page::Library);
    }

    #[test]
    fn test_global_key_2_switches_to_reader() {
        let mut state = test_state();
        state.current_page = Page::Library;
        let (tx, _rx) = mpsc::channel();
        key_handler::handle_key(&mut state, key_event(KEY_READER), &tx, Rect::default());
        assert_eq!(state.current_page, Page::Reader);
    }

    #[test]
    fn test_global_key_3_switches_to_settings() {
        let mut state = test_state();
        state.current_page = Page::Library;
        let (tx, _rx) = mpsc::channel();
        key_handler::handle_key(&mut state, key_event(KEY_SETTINGS), &tx, Rect::default());
        assert_eq!(state.current_page, Page::Settings);
    }

    #[test]
    fn test_colon_opens_command_palette() {
        let mut state = test_state();
        assert_eq!(state.modal, Modal::None);
        let (tx, _rx) = mpsc::channel();
        key_handler::handle_key(
            &mut state,
            key_event(KEY_COMMAND_PALETTE),
            &tx,
            Rect::default(),
        );
        assert!(matches!(state.modal, Modal::CommandPalette { .. }));
    }

    #[test]
    fn test_question_toggles_hints() {
        let mut state = test_state();
        let (tx, _rx) = mpsc::channel();
        let initial = state.show_hints;
        key_handler::handle_key(
            &mut state,
            key_event(KEY_TOGGLE_HINTS),
            &tx,
            Rect::default(),
        );
        assert_eq!(state.show_hints, !initial);
        key_handler::handle_key(
            &mut state,
            key_event(KEY_TOGGLE_HINTS),
            &tx,
            Rect::default(),
        );
        assert_eq!(state.show_hints, initial);
    }

    #[test]
    fn test_esc_closes_modal() {
        let mut state = test_state();
        state.modal = Modal::CommandPalette {
            query: "".into(),
            filtered: vec![],
            selected: 0,
        };
        state.current_page = Page::AddingBook;
        let (tx, _rx) = mpsc::channel();
        let result =
            key_handler::handle_key(&mut state, key_event(KEY_ESCAPE), &tx, Rect::default());
        assert!(result);
        assert_eq!(state.modal, Modal::None);
    }

    #[test]
    fn test_esc_quits_from_library() {
        let mut state = test_state();
        state.current_page = Page::Library;
        let (tx, _rx) = mpsc::channel();
        let result =
            key_handler::handle_key(&mut state, key_event(KEY_ESCAPE), &tx, Rect::default());
        assert!(!result);
    }

    #[test]
    fn test_esc_quits_from_any_page() {
        let mut state = test_state();
        state.current_page = Page::Settings;
        let (tx, _rx) = mpsc::channel();
        let result =
            key_handler::handle_key(&mut state, key_event(KEY_ESCAPE), &tx, Rect::default());
        assert!(!result);
    }

    #[test]
    fn test_colon_does_not_open_palette_when_modal_open() {
        let mut state = test_state();
        state.modal = Modal::AddBook {
            inputs: vec![String::new()],
            cursor: 0,
            scroll_offset: 0,
        };
        let (tx, _rx) = mpsc::channel();
        key_handler::handle_key(
            &mut state,
            key_event(KEY_COMMAND_PALETTE),
            &tx,
            Rect::default(),
        );
        assert!(matches!(state.modal, Modal::AddBook { .. }));
    }

    #[test]
    fn test_global_key_does_not_switch_page_when_modal_open() {
        let mut state = test_state();
        state.current_page = Page::Library;
        state.modal = Modal::AddBook {
            inputs: vec![String::new()],
            cursor: 0,
            scroll_offset: 0,
        };
        let (tx, _rx) = mpsc::channel();
        key_handler::handle_key(&mut state, key_event(KEY_READER), &tx, Rect::default());
        assert_eq!(state.current_page, Page::Library);
    }
}
