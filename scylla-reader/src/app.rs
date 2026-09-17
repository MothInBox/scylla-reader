//! Owns the terminal, channels, state, and main loop. Bridges SSE server
//! events, input dispatch, cover loading, and UI rendering.

use crate::event;
use crate::event_types::ServerEvent;
use crate::key_handler;
use crate::state::AppState;
use crate::ui;
use scylla_core::types::JobDto;

use crossterm::{
    ExecutableCommand,
    event::Event,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use futures_util::StreamExt;
use ratatui::prelude::*;
use std::io::stdout;
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

pub struct App {
    terminal: Terminal<CrosstermBackend<Box<dyn std::io::Write>>>,
    state: AppState,
    event_rx: mpsc::Receiver<ServerEvent>,
    fetched_covers: std::collections::HashSet<String>,
    server_process: Option<Child>,
}

impl App {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let (event_tx, event_rx) = mpsc::channel::<ServerEvent>();
        // Install the global event sender so the one-shot AI search thread can
        // deliver `AiSearchResults` through the same channel.
        crate::event_types::set_event_tx(event_tx.clone());

        let terminal = Terminal::new(CrosstermBackend::new(
            Box::new(stdout()) as Box<dyn std::io::Write>
        ))?;

        let mut state = AppState::new();
        state.cover_picker = ratatui_image::picker::Picker::from_query_stdio()
            .unwrap_or_else(|_| ratatui_image::picker::Picker::from_fontsize((8, 12)));

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

        let base_url = state
            .lib
            .manager
            .primary_backend()
            .and_then(|b| b.url())
            .unwrap_or_else(|| "http://127.0.0.1:8080".to_string());

        Self::spawn_sse_client(base_url, event_tx);

        Ok(Self {
            terminal,
            state,
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

        // Redirect the server's stdout/stderr to a log file so its output
        // (e.g. the embedder's "Loading embedding model..." message) never
        // corrupts the TUI's raw-mode terminal.
        let log_file = Self::server_log_file();

        let mut cmd = Self::server_command();
        cmd.arg(port.to_string());
        Self::redirect_server_output(&mut cmd, log_file.as_ref());
        let child = match cmd.spawn() {
            Ok(child) => child,
            Err(e) => {
                crate::settings::log(
                    crate::settings::LogLevel::Error,
                    "SERVER",
                    &format!("Failed to start scylla-server from resolved path: {}", e),
                );
                // The sibling binary may exist but not be executable — retry
                // once with a plain PATH lookup before giving up.
                let mut cmd = Command::new("scylla-server");
                cmd.arg(port.to_string());
                Self::redirect_server_output(&mut cmd, log_file.as_ref());
                match cmd.spawn() {
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

    /// Open (append) the server log file at `{data_dir}/scylla-reader/server.log`,
    /// creating the directory if needed. Returns `None` (and logs) on failure —
    /// the server then inherits the TUI's stdio as before.
    fn server_log_file() -> Option<std::fs::File> {
        let data_dir = dirs::data_local_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
        let log_path = data_dir.join("scylla-reader").join("server.log");
        if let Some(parent) = log_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
        {
            Ok(file) => {
                crate::settings::log(
                    crate::settings::LogLevel::Debug,
                    "SERVER",
                    &format!("Server log: {}", log_path.display()),
                );
                Some(file)
            }
            Err(e) => {
                crate::settings::log(
                    crate::settings::LogLevel::Error,
                    "SERVER",
                    &format!("Failed to open server log {}: {}", log_path.display(), e),
                );
                None
            }
        }
    }

    /// Point the command's stdout and stderr at the server log file (two
    /// independent handles to the same file).
    fn redirect_server_output(cmd: &mut Command, log_file: Option<&std::fs::File>) {
        let Some(file) = log_file else {
            return;
        };
        match file.try_clone() {
            Ok(stdout) => {
                cmd.stdout(Stdio::from(stdout));
                match file.try_clone() {
                    Ok(stderr) => {
                        cmd.stderr(Stdio::from(stderr));
                    }
                    Err(e) => crate::settings::log(
                        crate::settings::LogLevel::Error,
                        "SERVER",
                        &format!("Failed to clone server log handle for stderr: {}", e),
                    ),
                }
            }
            Err(e) => crate::settings::log(
                crate::settings::LogLevel::Error,
                "SERVER",
                &format!("Failed to clone server log handle for stdout: {}", e),
            ),
        }
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

    /// Spawn the SSE client thread: subscribes to `GET /api/jobs/stream`,
    /// fetches the job snapshot, and forwards every event into `event_tx`.
    /// Reconnects forever with exponential backoff (1s → 10s max), and exits
    /// when the event receiver is dropped (TUI shutdown).
    fn spawn_sse_client(base_url: String, event_tx: mpsc::Sender<ServerEvent>) {
        std::thread::Builder::new()
            .name("sse-client".to_string())
            .spawn(move || {
                let runtime =
                    tokio::runtime::Runtime::new().expect("failed to create tokio runtime");
                let mut backoff = Duration::from_secs(1);
                loop {
                    match runtime.block_on(sse_loop(&base_url, &event_tx)) {
                        SseExit::Shutdown => break,
                        SseExit::Connected => {
                            let _ =
                                event_tx.send(ServerEvent::ConnectionState { connected: false });
                            backoff = Duration::from_secs(1);
                        }
                        SseExit::Disconnected => {
                            let _ =
                                event_tx.send(ServerEvent::ConnectionState { connected: false });
                        }
                    }
                    std::thread::sleep(backoff);
                    backoff = (backoff * 2).min(Duration::from_secs(10));
                }
            })
            .expect("failed to spawn sse client thread");
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
            event::update_covers(&mut self.state, &mut self.fetched_covers);
            event::update_embedding_statuses(&mut self.state);

            let area = self.draw()?;

            if crossterm::event::poll(Duration::from_millis(16))?
                && let Event::Key(key) = crossterm::event::read()?
                && !key_handler::handle_key(&mut self.state, key, area)
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
        let (_event_tx, event_rx) = mpsc::channel::<ServerEvent>();
        Self {
            terminal,
            state,
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

/// Outcome of one SSE connection cycle.
enum SseExit {
    /// Stream was established and then ended/errored — reconnect with a reset backoff.
    Connected,
    /// Never connected — reconnect with a growing backoff.
    Disconnected,
    /// The event receiver is gone (TUI exited) — stop the thread.
    Shutdown,
}

/// One SSE connection cycle: subscribe first, buffer events while the job
/// snapshot is fetched, apply the snapshot, flush the buffer, then stream
/// normally until the stream ends or errors.
async fn sse_loop(base_url: &str, event_tx: &mpsc::Sender<ServerEvent>) -> SseExit {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .expect("failed to build http client");

    // 1. Subscribe to the SSE stream FIRST so no events are missed while the
    //    snapshot is fetched.
    let resp = match client
        .get(format!("{}/api/jobs/stream", base_url))
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => resp,
        Ok(resp) => {
            crate::settings::log(
                crate::settings::LogLevel::Error,
                "SSE",
                &format!("SSE stream HTTP {}", resp.status()),
            );
            return SseExit::Disconnected;
        }
        Err(e) => {
            crate::settings::log(
                crate::settings::LogLevel::Error,
                "SSE",
                &format!("Failed to subscribe to SSE stream: {}", e),
            );
            return SseExit::Disconnected;
        }
    };

    // 2. Mark connected.
    if event_tx
        .send(ServerEvent::ConnectionState { connected: true })
        .is_err()
    {
        return SseExit::Shutdown;
    }

    let mut stream = resp.bytes_stream();
    let mut buffer: Vec<u8> = Vec::new();
    let mut pending: Vec<ServerEvent> = Vec::new();
    let mut snapshot_applied = false;

    // Fetch the snapshot concurrently with the stream read.
    let snapshot_fut = client.get(format!("{}/api/jobs", base_url)).send();
    tokio::pin!(snapshot_fut);

    loop {
        tokio::select! {
            chunk = stream.next() => {
                match chunk {
                    Some(Ok(bytes)) => {
                        buffer.extend_from_slice(&bytes);
                        while let Some(pos) = buffer.iter().position(|&b| b == b'\n') {
                            let line: Vec<u8> = buffer.drain(..=pos).collect();
                            let line = String::from_utf8_lossy(&line);
                            let line = line.trim();
                            // `data: <json>` lines carry events; `:` comment
                            // lines are keepalives and are ignored.
                            if let Some(data) = line.strip_prefix("data:") {
                                let data = data.trim();
                                if !data.is_empty() {
                                    match serde_json::from_str::<ServerEvent>(data) {
                                        Ok(event) => {
                                            if snapshot_applied {
                                                if event_tx.send(event).is_err() {
                                                    return SseExit::Shutdown;
                                                }
                                            } else {
                                                pending.push(event);
                                            }
                                        }
                                        Err(e) => crate::settings::log(
                                            crate::settings::LogLevel::Error,
                                            "SSE",
                                            &format!("Failed to parse SSE event: {}", e),
                                        ),
                                    }
                                }
                            }
                        }
                    }
                    Some(Err(e)) => {
                        crate::settings::log(
                            crate::settings::LogLevel::Error,
                            "SSE",
                            &format!("SSE stream error: {}", e),
                        );
                        return SseExit::Connected;
                    }
                    None => {
                        // Stream ended — flush any buffered events so they
                        // aren't lost, then reconnect.
                        for event in pending.drain(..) {
                            if event_tx.send(event).is_err() {
                                return SseExit::Shutdown;
                            }
                        }
                        return SseExit::Connected;
                    }
                }
            }
            snapshot = &mut snapshot_fut, if !snapshot_applied => {
                match snapshot {
                    Ok(resp) if resp.status().is_success() => {
                        match resp.json::<serde_json::Value>().await {
                            Ok(json) => {
                                let server_now_ms = json
                                    .get("server_now_ms")
                                    .and_then(|v| v.as_u64())
                                    .unwrap_or(0);
                                let jobs: Vec<JobDto> = json
                                    .get("jobs")
                                    .and_then(|j| j.as_array())
                                    .map(|arr| {
                                        let mut dropped = 0;
                                        let jobs: Vec<JobDto> = arr
                                            .iter()
                                            .filter_map(|v| {
                                                match serde_json::from_value(v.clone()) {
                                                    Ok(job) => Some(job),
                                                    Err(_) => {
                                                        dropped += 1;
                                                        None
                                                    }
                                                }
                                            })
                                            .collect();
                                        if dropped > 0 {
                                            crate::settings::log(
                                                crate::settings::LogLevel::Error,
                                                "SSE",
                                                &format!(
                                                    "Dropped {} malformed job(s) from snapshot",
                                                    dropped
                                                ),
                                            );
                                        }
                                        jobs
                                    })
                                    .unwrap_or_default();
                                if event_tx
                                    .send(ServerEvent::JobsSnapshot { jobs, server_now_ms })
                                    .is_err()
                                {
                                    return SseExit::Shutdown;
                                }
                            }
                            Err(e) => crate::settings::log(
                                crate::settings::LogLevel::Error,
                                "SSE",
                                &format!("Failed to parse jobs snapshot: {}", e),
                            ),
                        }
                    }
                    Ok(resp) => {
                        crate::settings::log(
                            crate::settings::LogLevel::Error,
                            "SSE",
                            &format!("Jobs snapshot HTTP {}", resp.status()),
                        );
                    }
                    Err(e) => {
                        crate::settings::log(
                            crate::settings::LogLevel::Error,
                            "SSE",
                            &format!("Failed to fetch jobs snapshot: {}", e),
                        );
                    }
                }
                snapshot_applied = true;
                // Flush events that arrived while the snapshot was in flight.
                // The JobEnqueued dedupe in event.rs skips ids already in the
                // snapshot.
                for event in pending.drain(..) {
                    if event_tx.send(event).is_err() {
                        return SseExit::Shutdown;
                    }
                }
            }
        }
    }
}

/// Resolve which `scylla-server` binary to launch. If `exe_dir` contains a
/// `scylla-server` file, use it; otherwise fall back to a PATH lookup.
fn resolve_server_binary(exe_dir: Option<&std::path::Path>) -> std::path::PathBuf {
    if let Some(dir) = exe_dir {
        // Prefer the release sibling (e.g. target/debug/../release/scylla-server)
        // — the embedding inference is far slower in debug builds.
        if let Some(parent) = dir.parent() {
            let release = parent.join("release").join("scylla-server");
            if release.is_file() {
                return release;
            }
        }
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
        key_handler::handle_key(&mut state, key_event(KEY_LIBRARY), Rect::default());
        assert_eq!(state.ui.page, Page::Library);
    }

    #[test]
    fn test_global_key_2_switches_to_reader() {
        let mut state = test_state();
        state.ui.page = Page::Library;
        key_handler::handle_key(&mut state, key_event(KEY_READER), Rect::default());
        assert_eq!(state.ui.page, Page::Reader);
    }

    #[test]
    fn test_global_key_3_switches_to_settings() {
        let mut state = test_state();
        state.ui.page = Page::Library;
        key_handler::handle_key(&mut state, key_event(KEY_SETTINGS), Rect::default());
        assert_eq!(state.ui.page, Page::Settings);
    }

    #[test]
    fn test_colon_opens_command_palette() {
        let mut state = test_state();
        assert_eq!(state.ui.modal, Modal::None);
        key_handler::handle_key(&mut state, key_event(KEY_COMMAND_PALETTE), Rect::default());
        assert!(matches!(state.ui.modal, Modal::CommandPalette { .. }));
    }

    #[test]
    fn test_question_toggles_hints() {
        let mut state = test_state();
        let initial = state.ui.show_hints;
        key_handler::handle_key(&mut state, key_event(KEY_TOGGLE_HINTS), Rect::default());
        assert_eq!(state.ui.show_hints, !initial);
        key_handler::handle_key(&mut state, key_event(KEY_TOGGLE_HINTS), Rect::default());
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
        let result = key_handler::handle_key(&mut state, key_event(KEY_ESCAPE), Rect::default());
        assert!(result);
        assert_eq!(state.ui.modal, Modal::None);
    }

    #[test]
    fn test_esc_quits_from_library() {
        let mut state = test_state();
        state.ui.page = Page::Library;
        let result = key_handler::handle_key(&mut state, key_event(KEY_ESCAPE), Rect::default());
        assert!(!result);
    }

    #[test]
    fn test_esc_quits_from_any_page() {
        let mut state = test_state();
        state.ui.page = Page::Settings;
        let result = key_handler::handle_key(&mut state, key_event(KEY_ESCAPE), Rect::default());
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
        key_handler::handle_key(&mut state, key_event(KEY_COMMAND_PALETTE), Rect::default());
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
        key_handler::handle_key(&mut state, key_event(KEY_READER), Rect::default());
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
    fn test_resolve_server_binary_prefers_release_sibling() {
        // exe_dir = target/debug → the release sibling target/release/scylla-server
        // must win over the debug sibling.
        let dir = tempfile::tempdir().unwrap();
        let debug_dir = dir.path().join("debug");
        let release_dir = dir.path().join("release");
        std::fs::create_dir_all(&debug_dir).unwrap();
        std::fs::create_dir_all(&release_dir).unwrap();
        let debug_bin = debug_dir.join("scylla-server");
        let release_bin = release_dir.join("scylla-server");
        std::fs::write(&debug_bin, "").unwrap();
        std::fs::write(&release_bin, "").unwrap();
        assert_eq!(resolve_server_binary(Some(&debug_dir)), release_bin);
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
