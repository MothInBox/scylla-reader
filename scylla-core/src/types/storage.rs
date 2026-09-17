use crate::types::{Book, BookStatus, Session};
use async_trait::async_trait;

#[async_trait]
pub trait StorageBackend: Send + Sync {
    fn name(&self) -> &str;
    fn kind(&self) -> crate::types::filter::BackendKind;
    fn url(&self) -> Option<String> {
        None
    }

    async fn list_books(&self) -> Result<Vec<Book>, String>;
    async fn get_book(&self, url: &str) -> Result<Book, String>;
    async fn upsert_book(&self, book: &Book) -> Result<(), String>;
    async fn delete_book(&self, url: &str) -> Result<(), String>;
    async fn update_status(&self, url: &str, status: &BookStatus) -> Result<(), String>;
    async fn list_sessions(&self, book_url: &str) -> Result<Vec<Session>, String>;
    async fn create_session(
        &self,
        book_url: &str,
        name: &str,
        total: u32,
    ) -> Result<Session, String>;
    async fn rename_session(&self, session_id: i64, name: &str) -> Result<(), String>;
    async fn delete_session(&self, session_id: i64) -> Result<(), String>;
    async fn update_progress(&self, session_id: i64, current: u32) -> Result<(), String>;
    async fn set_active_session(&self, book_url: &str, session_id: i64) -> Result<(), String>;

    async fn get_server_settings(&self) -> Result<serde_json::Value, String> {
        Err("not supported".into())
    }
    async fn update_server_settings(&self, _settings: &serde_json::Value) -> Result<(), String> {
        Err("not supported".into())
    }
}
