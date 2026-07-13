use async_trait::async_trait;
use scylla_core::types::*;

pub struct RemoteApi {
    name: String,
    base_url: String,
    client: reqwest::Client,
}

impl RemoteApi {
    pub fn new(name: String, base_url: String) -> Self {
        Self {
            name,
            base_url,
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl StorageBackend for RemoteApi {
    fn name(&self) -> &str {
        &self.name
    }
    fn kind(&self) -> BackendKind {
        BackendKind::Remote
    }

    async fn list_books(&self) -> Result<Vec<Book>, String> {
        let resp = self
            .client
            .get(format!("{}/api/books", self.base_url))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        resp.json().await.map_err(|e| e.to_string())
    }

    async fn get_book(&self, url: &str) -> Result<Book, String> {
        let resp = self
            .client
            .get(format!("{}/api/books", self.base_url))
            .query(&[("url", url)])
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if resp.status().is_success() {
            resp.json().await.map_err(|e| e.to_string())
        } else {
            Err(format!("HTTP {}", resp.status()))
        }
    }

    async fn upsert_book(&self, book: &Book) -> Result<(), String> {
        let resp = self
            .client
            .post(format!("{}/api/books", self.base_url))
            .json(&serde_json::json!({ "url": book.url }))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(format!("HTTP {}", resp.status()))
        }
    }

    async fn delete_book(&self, url: &str) -> Result<(), String> {
        let resp = self
            .client
            .delete(format!("{}/api/books", self.base_url))
            .query(&[("url", url)])
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(format!("HTTP {}", resp.status()))
        }
    }

    async fn update_status(&self, url: &str, status: &BookStatus) -> Result<(), String> {
        let resp = self
            .client
            .patch(format!("{}/api/books/status", self.base_url))
            .query(&[("url", url)])
            .json(&serde_json::json!({ "status": status.to_string() }))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(format!("HTTP {}", resp.status()))
        }
    }

    async fn list_sessions(&self, book_url: &str) -> Result<Vec<Session>, String> {
        let resp = self
            .client
            .get(format!("{}/api/books/sessions", self.base_url))
            .query(&[("url", book_url)])
            .send()
            .await
            .map_err(|e| e.to_string())?;
        resp.json().await.map_err(|e| e.to_string())
    }

    async fn update_progress(&self, session_id: i64, current: u32) -> Result<(), String> {
        let resp = self
            .client
            .patch(format!(
                "{}/api/sessions/{}/progress",
                self.base_url, session_id
            ))
            .json(&serde_json::json!({ "current": current }))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(format!("HTTP {}", resp.status()))
        }
    }

    async fn set_active_session(&self, book_url: &str, session_id: i64) -> Result<(), String> {
        let resp = self
            .client
            .patch(format!("{}/api/books/active-session", self.base_url))
            .query(&[("url", book_url)])
            .json(&serde_json::json!({ "session_id": session_id }))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(format!("HTTP {}", resp.status()))
        }
    }

    async fn get_chapter_content(
        &self,
        book_url: &str,
        chapter_url: &str,
    ) -> Result<String, String> {
        let resp = self
            .client
            .get(format!("{}/api/chapters/content", self.base_url))
            .query(&[("book_url", book_url), ("chapter_url", chapter_url)])
            .send()
            .await
            .map_err(|e| e.to_string())?;
        let json: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
        Ok(json["content"].as_str().unwrap_or("").to_string())
    }

    async fn scrape_book(&self, url: &str) -> Result<Book, String> {
        let resp = self
            .client
            .post(format!("{}/api/books", self.base_url))
            .json(&serde_json::json!({ "url": url }))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        let status = resp.status();
        if status.is_success() {
            return resp.json().await.map_err(|e| e.to_string());
        }
        if status.as_u16() == 202 {
            for _ in 0..30 {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                let poll = self
                    .client
                    .get(format!("{}/api/books", self.base_url))
                    .query(&[("url", url)])
                    .send()
                    .await
                    .map_err(|e| e.to_string())?;
                if poll.status().is_success() {
                    return poll.json().await.map_err(|e| e.to_string());
                }
            }
            return Err("Timeout waiting for book scrape".to_string());
        }
        Err(format!("HTTP {}", status))
    }

    async fn fetch_chapter(
        &self,
        book_url: &str,
        chapter_url: &str,
    ) -> Result<ChapterContent, String> {
        let resp = self
            .client
            .get(format!("{}/api/chapters/content", self.base_url))
            .query(&[("book_url", book_url), ("chapter_url", chapter_url)])
            .send()
            .await
            .map_err(|e| e.to_string())?;
        let json: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
        Ok(ChapterContent {
            title: json["title"].as_str().unwrap_or("").to_string(),
            content: json["content"].as_str().unwrap_or("").to_string(),
        })
    }
}
