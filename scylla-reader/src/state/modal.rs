use crate::models::Chapter;

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
