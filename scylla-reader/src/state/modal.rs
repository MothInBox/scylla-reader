//! Modal state — add-book text inputs and jump-to-chapter list state.

use crate::models::Chapter;

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
}
