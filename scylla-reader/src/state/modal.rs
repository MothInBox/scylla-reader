//! Modal state — add-book text inputs, jump-to-chapter list state, and command palette.

use crate::messenger::AppCommand;
use crate::models::Chapter;
use crate::state::AppState;

#[derive(Debug, PartialEq)]
pub enum Modal {
    None,
    AddBook {
        inputs: Vec<String>,
        cursor: usize,
        scroll_offset: usize,
    },
    JumpChapter {
        chapters: Vec<Chapter>,
        cursor: usize,
        scroll_offset: usize,
        show_titles: bool,
    },
    SessionPicker {
        book_url: String,
        cursor: usize,
        scroll_offset: usize,
    },
    CommandPalette {
        query: String,
        filtered: Vec<PaletteAction>,
        selected: usize,
    },
}

#[derive(Debug, Clone)]
pub struct PaletteAction {
    pub category: &'static str,
    pub label: &'static str,
    pub keys: &'static str,
    pub handler: fn(&mut AppState, &std::sync::mpsc::Sender<AppCommand>),
}

impl PartialEq for PaletteAction {
    fn eq(&self, other: &Self) -> bool {
        self.category == other.category && self.label == other.label && self.keys == other.keys
    }
}
