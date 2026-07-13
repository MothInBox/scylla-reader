use async_trait::async_trait;
use crate::types::{Book, BookStatus, Session};

#[derive(Debug, Clone)]
pub struct ChapterContent {
    pub title: String,
    pub content: String,
}

#[async_trait]
pub trait StorageBackend: Send + Sync {
    fn name(&self) -> &str;
    fn kind(&self) -> crate::types::filter::BackendKind;

    async fn list_books(&self) -> Result<Vec<Book>, String>;
    async fn get_book(&self, url: &str) -> Result<Book, String>;
    async fn upsert_book(&self, book: &Book) -> Result<(), String>;
    async fn delete_book(&self, url: &str) -> Result<(), String>;
    async fn update_status(&self, url: &str, status: &BookStatus) -> Result<(), String>;
    async fn list_sessions(&self, book_url: &str) -> Result<Vec<Session>, String>;
    async fn update_progress(&self, session_id: i64, current: u32) -> Result<(), String>;
    async fn set_active_session(&self, book_url: &str, session_id: i64) -> Result<(), String>;
    async fn get_chapter_content(&self, book_url: &str, chapter_url: &str) -> Result<String, String>;
    async fn scrape_book(&self, url: &str) -> Result<Book, String>;
    async fn fetch_chapter(&self, book_url: &str, chapter_url: &str) -> Result<ChapterContent, String>;
}
