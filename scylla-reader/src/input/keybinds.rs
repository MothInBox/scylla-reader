use crossterm::event::{KeyCode, KeyModifiers};

// ── Global Navigation ──
pub const KEY_LIBRARY: KeyCode = KeyCode::Char('1');
pub const KEY_READER: KeyCode = KeyCode::Char('2');
pub const KEY_JOBS: KeyCode = KeyCode::Char('8');
pub const KEY_SETTINGS: KeyCode = KeyCode::Char('9');
pub const KEY_COMMAND_PALETTE: KeyCode = KeyCode::Char(':');
pub const KEY_TOGGLE_HINTS: KeyCode = KeyCode::Char('?');
pub const KEY_ESCAPE: KeyCode = KeyCode::Esc;

// ── Library Page ──
pub const KEY_ADD_BOOK: KeyCode = KeyCode::Char('i');
pub const KEY_JUMP_CHAPTER: KeyCode = KeyCode::Char('j');
pub const KEY_DELETE: KeyCode = KeyCode::Char('d');
pub const KEY_CYCLE_STATUS: KeyCode = KeyCode::Char(' ');
pub const KEY_CYCLE_FILTER: KeyCode = KeyCode::Char('f');
pub const KEY_UPDATE_ALL: KeyCode = KeyCode::Char('u');
pub const KEY_SESSIONS: KeyCode = KeyCode::Enter;
pub const KEY_NAV_UP: KeyCode = KeyCode::Up;
pub const KEY_NAV_DOWN: KeyCode = KeyCode::Down;

// ── Reader Page ──
pub const KEY_NEXT_CHAPTER: KeyCode = KeyCode::Char('>');
pub const KEY_PREV_CHAPTER: KeyCode = KeyCode::Char('<');
pub const KEY_NEXT_PAGE: KeyCode = KeyCode::Right;
pub const KEY_PREV_PAGE: KeyCode = KeyCode::Left;
pub const KEY_SCROLL_UP: KeyCode = KeyCode::Up;
pub const KEY_SCROLL_DOWN: KeyCode = KeyCode::Down;
pub const KEY_MANAGE_SESSIONS: KeyCode = KeyCode::Char('s');

// ── Modal: Add Book ──
pub const KEY_SUBMIT_MODIFIER: KeyModifiers = KeyModifiers::CONTROL;
pub const KEY_SUBMIT: KeyCode = KeyCode::Char('s');

// ── Modal: Jump Chapter ──
pub const KEY_TOGGLE_TITLES: KeyCode = KeyCode::Char('t');

// ── Modal: Session Picker ──
pub const KEY_NEW_SESSION: KeyCode = KeyCode::Char('n');
pub const KEY_RENAME_SESSION: KeyCode = KeyCode::Char('r');
pub const KEY_DELETE_SESSION: KeyCode = KeyCode::Char('d');

// ── Generic ──
pub const KEY_BACKSPACE: KeyCode = KeyCode::Backspace;
pub const KEY_ENTER: KeyCode = KeyCode::Enter;
