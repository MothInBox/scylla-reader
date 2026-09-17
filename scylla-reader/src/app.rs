//! Owns the terminal, channels, state, and main loop. Bridges worker events,
//! input dispatch, cover loading, and UI rendering.

use crate::event;
use crate::key_handler;
use crate::state::AppState;
use crate::ui;
use scylla_core::messenger::{AppCommand, AppEvent};
use scylla_core::scraper::ScraperRegistry;
use scylla_core::worker::JobManager;

use crossterm::{
    ExecutableCommand,
    event::Event,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::prelude::*;
use std::io::stdout;
use std::net::TcpStream;
use std::process::{Child, Command};
use std::sync::mpsc;
use std::time::Duration;

pub struct App {
    terminal: Terminal<CrosstermBackend<Box<dyn std::io::Write>>>,
    state: AppState,
    cmd_tx: mpsc::Sender<AppCommand>,
    event_rx: mpsc::Receiver<AppEvent>,
    fetched_covers: std::collections::HashSet<String>,
    server_process: Option<Child>,
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

        let server_process = Self::ensure_local_server();

        match state.lib.manager.primary_backend() {
            Some(backend) => match crate::storage::client::block_on(backend.list_books()) {
                Ok(books) => state.lib.library.books = books,
                Err(e) => crate::settings::log(
                    crate::settings::LogLevel::Error,
                    "UI",
                    &format!("Failed to load books from backend: {}", e),
                ),
            },
            None => crate::settings::log(
                crate::settings::LogLevel::Error,
                "UI",
                "No backend configured — persistence disabled",
            ),
        }

        let max_workers = state.lib.settings.max_workers;
        let rate_limit = state.lib.settings.rate_limit_secs;

        Self::spawn_worker_thread(
            cmd_rx,
            event_tx,
            registry,
            picker_font_size,
            picker_protocol_type,
            max_workers,
            rate_limit,
        );

        Ok(Self {
            terminal,
            state,
            cmd_tx,
            event_rx,
            fetched_covers: std::collections::HashSet::new(),
            server_process,
        })
    }

    fn ensure_local_server() -> Option<Child> {
        let port = 8080;
        let addr: std::net::SocketAddr = format!("127.0.0.1:{}", port)
            .parse()
            .unwrap_or_else(|_| ([127, 0, 0, 1], port).into());

        if TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_ok() {
            crate::settings::log(
                crate::settings::LogLevel::Debug,
                "SERVER",
                "Server already running",
            );
            return None;
        }

        let child = match Self::server_command().arg(port.to_string()).spawn() {
            Ok(child) => child,
            Err(e) => {
                crate::settings::log(
                    crate::settings::LogLevel::Error,
                    "SERVER",
                    &format!("Failed to start scylla-server from resolved path: {}", e),
                );
                // The sibling binary may exist but not be executable — retry
                // once with a plain PATH lookup before giving up.
                match Command::new("scylla-server").arg(port.to_string()).spawn() {
                    Ok(child) => child,
                    Err(e) => {
                        crate::settings::log(
                            crate::settings::LogLevel::Error,
                            "SERVER",
                            &format!("Failed to start scylla-server from PATH: {}", e),
                        );
                        return None;
                    }
                }
            }
        };

        for i in 0..25 {
            std::thread::sleep(Duration::from_millis(200));
            if TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_ok() {
                crate::settings::log(
                    crate::settings::LogLevel::Debug,
                    "SERVER",
                    &format!("Server started after {} polls", i + 1),
                );
                return Some(child);
            }
        }

        let mut child = child;
        crate::settings::log(
            crate::settings::LogLevel::Error,
            "SERVER",
            "Server failed to start within timeout",
        );
        let _ = child.kill();
        let _ = child.wait();
        None
    }

    /// Build the command used to launch the local server. Prefers a
    /// `scylla-server` binary next to the current executable (so plain
    /// `cargo run -p scylla-reader` finds the sibling binary), falling back to
    /// a PATH lookup.
    fn server_command() -> Command {
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|e| e.parent().map(|p| p.to_path_buf()));
        let binary = resolve_server_binary(exe_dir.as_deref());
        crate::settings::log(
            crate::settings::LogLevel::Debug,
            "SERVER",
            &format!("Using server binary: {}", binary.display()),
        );
        Command::new(binary)
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
                let manager = JobManager::new(
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
            event::drain_events(&mut self.state, &self.event_rx);
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
            server_process: None,
        }
    }
}

impl Drop for App {
    fn drop(&mut self) {
        if let Some(mut child) = self.server_process.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Resolve which `scylla-server` binary to launch. If `exe_dir` contains a
/// `scylla-server` file, use it; otherwise fall back to a PATH lookup.
fn resolve_server_binary(exe_dir: Option<&std::path::Path>) -> std::path::PathBuf {
    if let Some(dir) = exe_dir {
        let candidate = dir.join("scylla-server");
        if candidate.is_file() {
            return candidate;
        }
    }
    std::path::PathBuf::from("scylla-server")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::keybinds::*;
    use crate::key_handler;
    use crate::library::Library;
    use crate::state::{Modal, Page};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn test_state() -> AppState {
        AppState::from_parts(Library::new())
    }

    fn key_event(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn test_global_key_1_switches_to_library() {
        let mut state = test_state();
        state.ui.page = Page::Reader;
        let (tx, _rx) = mpsc::channel();
        key_handler::handle_key(&mut state, key_event(KEY_LIBRARY), &tx, Rect::default());
        assert_eq!(state.ui.page, Page::Library);
    }

    #[test]
    fn test_global_key_2_switches_to_reader() {
        let mut state = test_state();
        state.ui.page = Page::Library;
        let (tx, _rx) = mpsc::channel();
        key_handler::handle_key(&mut state, key_event(KEY_READER), &tx, Rect::default());
        assert_eq!(state.ui.page, Page::Reader);
    }

    #[test]
    fn test_global_key_3_switches_to_settings() {
        let mut state = test_state();
        state.ui.page = Page::Library;
        let (tx, _rx) = mpsc::channel();
        key_handler::handle_key(&mut state, key_event(KEY_SETTINGS), &tx, Rect::default());
        assert_eq!(state.ui.page, Page::Settings);
    }

    #[test]
    fn test_colon_opens_command_palette() {
        let mut state = test_state();
        assert_eq!(state.ui.modal, Modal::None);
        let (tx, _rx) = mpsc::channel();
        key_handler::handle_key(
            &mut state,
            key_event(KEY_COMMAND_PALETTE),
            &tx,
            Rect::default(),
        );
        assert!(matches!(state.ui.modal, Modal::CommandPalette { .. }));
    }

    #[test]
    fn test_question_toggles_hints() {
        let mut state = test_state();
        let (tx, _rx) = mpsc::channel();
        let initial = state.ui.show_hints;
        key_handler::handle_key(
            &mut state,
            key_event(KEY_TOGGLE_HINTS),
            &tx,
            Rect::default(),
        );
        assert_eq!(state.ui.show_hints, !initial);
        key_handler::handle_key(
            &mut state,
            key_event(KEY_TOGGLE_HINTS),
            &tx,
            Rect::default(),
        );
        assert_eq!(state.ui.show_hints, initial);
    }

    #[test]
    fn test_esc_closes_modal() {
        let mut state = test_state();
        state.ui.modal = Modal::CommandPalette {
            query: "".into(),
            filtered: vec![],
            selected: 0,
        };
        let (tx, _rx) = mpsc::channel();
        let result =
            key_handler::handle_key(&mut state, key_event(KEY_ESCAPE), &tx, Rect::default());
        assert!(result);
        assert_eq!(state.ui.modal, Modal::None);
    }

    #[test]
    fn test_esc_quits_from_library() {
        let mut state = test_state();
        state.ui.page = Page::Library;
        let (tx, _rx) = mpsc::channel();
        let result =
            key_handler::handle_key(&mut state, key_event(KEY_ESCAPE), &tx, Rect::default());
        assert!(!result);
    }

    #[test]
    fn test_esc_quits_from_any_page() {
        let mut state = test_state();
        state.ui.page = Page::Settings;
        let (tx, _rx) = mpsc::channel();
        let result =
            key_handler::handle_key(&mut state, key_event(KEY_ESCAPE), &tx, Rect::default());
        assert!(!result);
    }

    #[test]
    fn test_colon_does_not_open_palette_when_modal_open() {
        let mut state = test_state();
        state.ui.modal = Modal::AddBook {
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
        assert!(matches!(state.ui.modal, Modal::AddBook { .. }));
    }

    #[test]
    fn test_global_key_does_not_switch_page_when_modal_open() {
        let mut state = test_state();
        state.ui.page = Page::Library;
        state.ui.modal = Modal::AddBook {
            inputs: vec![String::new()],
            cursor: 0,
            scroll_offset: 0,
        };
        let (tx, _rx) = mpsc::channel();
        key_handler::handle_key(&mut state, key_event(KEY_READER), &tx, Rect::default());
        assert_eq!(state.ui.page, Page::Library);
    }

    #[test]
    fn test_resolve_server_binary_prefers_sibling() {
        let dir = tempfile::tempdir().unwrap();
        let fake = dir.path().join("scylla-server");
        std::fs::write(&fake, "").unwrap();
        assert_eq!(resolve_server_binary(Some(dir.path())), fake);
    }

    #[test]
    fn test_resolve_server_binary_falls_back_to_path_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            resolve_server_binary(Some(dir.path())),
            std::path::PathBuf::from("scylla-server")
        );
    }

    #[test]
    fn test_resolve_server_binary_falls_back_to_path_without_exe_dir() {
        assert_eq!(
            resolve_server_binary(None),
            std::path::PathBuf::from("scylla-server")
        );
    }
}
