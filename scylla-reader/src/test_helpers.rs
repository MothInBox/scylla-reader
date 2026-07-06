//! Shared test utilities — all test modules should import from here to avoid duplication.

#![cfg(test)]

use crate::db::Db;
use crate::library::Library;
use crate::messenger::AppCommand;
use crate::state::AppState;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::prelude::Rect;
use std::sync::mpsc;

pub fn test_state() -> AppState {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    let db = Db::open_conn(conn).unwrap();
    AppState::from_parts(db, Library::new())
}

pub fn key_event(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

pub fn channel() -> (mpsc::Sender<AppCommand>, mpsc::Receiver<AppCommand>) {
    mpsc::channel()
}

pub fn rect() -> Rect {
    Rect::default()
}
