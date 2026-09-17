//! Shared test utilities — all test modules should import from here to avoid duplication.

#![cfg(test)]

use crate::library::Library;
use crate::settings::Settings;
use crate::state::settings_ui::SettingsUiState;
use crate::state::{AppState, JobsState, LibraryState, ReaderState, UiState};
use crate::storage::manager::LibraryManager;
use crate::storage::server_settings::ServerSettingsState;
use async_trait::async_trait;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::prelude::Rect;
use scylla_core::types::{BackendKind, Book, BookStatus, Progress, Session, StorageBackend};
use std::sync::{Arc, Mutex};

pub fn test_state() -> AppState {
    AppState::from_parts(Library::new())
}

/// Build an `AppState` whose library manager contains the given backend, so
/// persistence calls made by the TUI can be observed through the mock.
pub fn test_state_with_backend(backend: Box<dyn StorageBackend>) -> AppState {
    test_state_with_backends(vec![backend])
}

/// Build an `AppState` whose library manager contains several backends.
pub fn test_state_with_backends(backends: Vec<Box<dyn StorageBackend>>) -> AppState {
    let mut state = AppState {
        ui: UiState::new(),
        lib: LibraryState {
            library: Library::new(),
            manager: LibraryManager::new(backends),
            settings: Settings::new(),
            settings_ui: SettingsUiState::new(),
            server_settings: ServerSettingsState::new(),
        },
        reader: ReaderState::new(),
        jobs: JobsState::new(),
        cover_picker: ratatui_image::picker::Picker::from_fontsize((8, 12)),
    };
    state.jobs.max_workers = state.lib.settings.max_workers;
    state
}

pub fn key_event(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

pub fn rect() -> Rect {
    Rect::new(0, 0, 80, 24)
}

/// In-memory `StorageBackend` that records every persistence call so tests can
/// assert what the TUI sent to the server.
pub struct MockBackend {
    pub name: String,
    pub books: Arc<Mutex<Vec<Book>>>,
    pub sessions: Arc<Mutex<Vec<Session>>>,
    pub calls: Arc<Mutex<Vec<String>>>,
    pub server_settings: Arc<Mutex<Option<serde_json::Value>>>,
    fail: Arc<Mutex<bool>>,
}

impl MockBackend {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            books: Arc::new(Mutex::new(Vec::new())),
            sessions: Arc::new(Mutex::new(Vec::new())),
            calls: Arc::new(Mutex::new(Vec::new())),
            server_settings: Arc::new(Mutex::new(None)),
            fail: Arc::new(Mutex::new(false)),
        }
    }

    /// When set, mutating methods return `Err` instead of applying changes.
    pub fn set_fail(&self, fail: bool) {
        *self.fail.lock().unwrap() = fail;
    }

    fn should_fail(&self) -> bool {
        *self.fail.lock().unwrap()
    }

    fn record(&self, call: String) {
        self.calls.lock().unwrap().push(call);
    }
}

#[async_trait]
impl StorageBackend for MockBackend {
    fn name(&self) -> &str {
        &self.name
    }

    fn kind(&self) -> BackendKind {
        BackendKind::Remote
    }

    fn url(&self) -> Option<String> {
        Some("http://mock".to_string())
    }

    async fn list_books(&self) -> Result<Vec<Book>, String> {
        self.record("list_books".to_string());
        if self.should_fail() {
            return Err("mock failure".to_string());
        }
        Ok(self.books.lock().unwrap().clone())
    }

    async fn get_book(&self, url: &str) -> Result<Book, String> {
        self.books
            .lock()
            .unwrap()
            .iter()
            .find(|b| b.url == url)
            .cloned()
            .ok_or_else(|| "not found".to_string())
    }

    async fn upsert_book(&self, book: &Book) -> Result<(), String> {
        self.record(format!("upsert:{}", book.url));
        if self.should_fail() {
            return Err("mock failure".to_string());
        }
        let mut books = self.books.lock().unwrap();
        if let Some(existing) = books.iter_mut().find(|b| b.url == book.url) {
            *existing = book.clone();
        } else {
            books.push(book.clone());
        }
        drop(books);
        // Mirror the server's `ensure_default_session`: a book with no sessions
        // gets an "Initial" session.
        let mut sessions = self.sessions.lock().unwrap();
        if !sessions.iter().any(|s| s.book_url == book.url) {
            let id = 100 + sessions.len() as i64;
            sessions.push(Session {
                id,
                book_url: book.url.clone(),
                name: "Initial".into(),
                progress: Progress {
                    current: 0,
                    total: book.chapters.len() as u32,
                },
                created_at: String::new(),
                updated_at: String::new(),
            });
        }
        Ok(())
    }

    async fn delete_book(&self, url: &str) -> Result<(), String> {
        self.record(format!("delete:{}", url));
        if self.should_fail() {
            return Err("mock failure".to_string());
        }
        self.books.lock().unwrap().retain(|b| b.url != url);
        Ok(())
    }

    async fn update_status(&self, url: &str, status: &BookStatus) -> Result<(), String> {
        self.record(format!("update_status:{}:{}", url, status));
        if self.should_fail() {
            return Err("mock failure".to_string());
        }
        if let Some(book) = self.books.lock().unwrap().iter_mut().find(|b| b.url == url) {
            book.status = status.clone();
        }
        Ok(())
    }

    async fn list_sessions(&self, book_url: &str) -> Result<Vec<Session>, String> {
        self.record(format!("list_sessions:{}", book_url));
        Ok(self
            .sessions
            .lock()
            .unwrap()
            .iter()
            .filter(|s| s.book_url == book_url)
            .cloned()
            .collect())
    }

    async fn create_session(
        &self,
        book_url: &str,
        name: &str,
        total: u32,
    ) -> Result<Session, String> {
        self.record(format!("create_session:{}:{}", book_url, name));
        if self.should_fail() {
            return Err("mock failure".to_string());
        }
        let mut sessions = self.sessions.lock().unwrap();
        let session = Session {
            id: 100 + sessions.len() as i64,
            book_url: book_url.to_string(),
            name: name.to_string(),
            progress: Progress { current: 0, total },
            created_at: String::new(),
            updated_at: String::new(),
        };
        sessions.push(session.clone());
        Ok(session)
    }

    async fn rename_session(&self, session_id: i64, name: &str) -> Result<(), String> {
        self.record(format!("rename_session:{}", session_id));
        if self.should_fail() {
            return Err("mock failure".to_string());
        }
        if let Some(session) = self
            .sessions
            .lock()
            .unwrap()
            .iter_mut()
            .find(|s| s.id == session_id)
        {
            session.name = name.to_string();
        }
        Ok(())
    }

    async fn delete_session(&self, session_id: i64) -> Result<(), String> {
        self.record(format!("delete_session:{}", session_id));
        if self.should_fail() {
            return Err("mock failure".to_string());
        }
        self.sessions.lock().unwrap().retain(|s| s.id != session_id);
        Ok(())
    }

    async fn update_progress(&self, session_id: i64, current: u32) -> Result<(), String> {
        self.record(format!("update_progress:{}:{}", session_id, current));
        if self.should_fail() {
            return Err("mock failure".to_string());
        }
        if let Some(session) = self
            .sessions
            .lock()
            .unwrap()
            .iter_mut()
            .find(|s| s.id == session_id)
        {
            session.progress.current = current;
        }
        Ok(())
    }

    async fn set_active_session(&self, book_url: &str, session_id: i64) -> Result<(), String> {
        self.record(format!("set_active_session:{}:{}", book_url, session_id));
        if self.should_fail() {
            return Err("mock failure".to_string());
        }
        Ok(())
    }

    async fn get_server_settings(&self) -> Result<serde_json::Value, String> {
        self.server_settings
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| "not configured".to_string())
    }
}
