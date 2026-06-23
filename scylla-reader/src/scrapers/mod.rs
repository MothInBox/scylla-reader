pub mod cookie_store;
pub mod services;
pub use services::ScraperRegistry;


use crate::models::Book;
use async_trait::async_trait;

#[async_trait]
pub trait BookScraper: Send + Sync {
    async fn scrape(&self, url: &str) -> Result<Book, Box<dyn std::error::Error>>;
}
