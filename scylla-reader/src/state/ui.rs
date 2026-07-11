//! UI state — navigation, modals, hints.

use crate::state::modal::Modal;
use crate::state::page::Page;

pub struct UiState {
    pub page: Page,
    pub modal: Modal,
    pub show_hints: bool,
}

impl UiState {
    pub fn new() -> Self {
        Self {
            page: Page::Library,
            modal: Modal::None,
            show_hints: true,
        }
    }

    pub fn close_modal(&mut self) {
        self.modal = Modal::None;
    }
}

impl Default for UiState {
    fn default() -> Self {
        Self::new()
    }
}
