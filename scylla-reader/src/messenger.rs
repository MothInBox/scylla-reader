pub enum AppCommand {
    Scrape(String),
    UpdateAll(Vec<String>),
    FetchChapter(String, usize),
}

pub struct ChapterContent {
    pub chapter_idx: usize,
    pub title: String,
    pub content: String,
}
