use crate::models::Book;

pub enum AppCommand {
    Scrape(String),
    UpdateAll(Vec<String>),
    FetchChapter(String, usize), // (chapter_url, chapter_idx)
}

pub enum AppEvent {
    BookAdded(Book),
    BookUpdated(Book),
    Error(String),
}

pub struct ChapterContent {
    pub chapter_idx: usize,
    pub title: String,
    pub content: String,
}
