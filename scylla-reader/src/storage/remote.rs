use async_trait::async_trait;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use scylla_core::types::*;

/// Percent-encode set for a single URL path segment: everything except the
/// RFC 3986 unreserved characters (`ALPHA / DIGIT / "-" / "." / "_" / "~"`).
const PATH_SEGMENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

fn encode_segment(value: &str) -> String {
    utf8_percent_encode(value, PATH_SEGMENT).to_string()
}

fn book_url(base: &str, url: &str) -> String {
    format!("{}/api/books/{}", base, encode_segment(url))
}

fn status_url(base: &str, book_url: &str) -> String {
    format!("{}/api/books/{}/status", base, encode_segment(book_url))
}

fn sessions_url(base: &str, book_url: &str) -> String {
    format!("{}/api/books/{}/sessions", base, encode_segment(book_url))
}

fn active_session_url(base: &str, book_url: &str) -> String {
    format!(
        "{}/api/books/{}/active-session",
        base,
        encode_segment(book_url)
    )
}

fn session_url(base: &str, session_id: i64) -> String {
    format!("{}/api/sessions/{}", base, session_id)
}

pub struct RemoteApi {
    name: String,
    base_url: String,
    client: reqwest::Client,
}

impl RemoteApi {
    pub fn new(name: String, base_url: String) -> Self {
        Self {
            name,
            base_url: base_url.trim_end_matches('/').to_string(),
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(5))
                .build()
                .expect("failed to build http client"),
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
    fn url(&self) -> Option<String> {
        Some(self.base_url.clone())
    }

    async fn list_books(&self) -> Result<Vec<Book>, String> {
        let resp = self
            .client
            .get(format!("{}/api/books", self.base_url))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if resp.status().is_success() {
            resp.json().await.map_err(|e| e.to_string())
        } else {
            Err(format!("HTTP {}", resp.status()))
        }
    }

    async fn get_book(&self, url: &str) -> Result<Book, String> {
        let resp = self
            .client
            .get(book_url(&self.base_url, url))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        match resp.status().as_u16() {
            404 => Err("not found".to_string()),
            status if (200..300).contains(&status) => resp.json().await.map_err(|e| e.to_string()),
            status => Err(format!("HTTP {}", status)),
        }
    }

    async fn upsert_book(&self, book: &Book) -> Result<(), String> {
        let resp = self
            .client
            .put(book_url(&self.base_url, &book.url))
            .json(book)
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
            .delete(book_url(&self.base_url, url))
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
            .patch(status_url(&self.base_url, url))
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
            .get(sessions_url(&self.base_url, book_url))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if resp.status().is_success() {
            resp.json().await.map_err(|e| e.to_string())
        } else {
            Err(format!("HTTP {}", resp.status()))
        }
    }

    async fn create_session(
        &self,
        book_url: &str,
        name: &str,
        total: u32,
    ) -> Result<Session, String> {
        let resp = self
            .client
            .post(sessions_url(&self.base_url, book_url))
            .json(&serde_json::json!({ "name": name, "total": total }))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if resp.status().is_success() {
            resp.json().await.map_err(|e| e.to_string())
        } else {
            Err(format!("HTTP {}", resp.status()))
        }
    }

    async fn rename_session(&self, session_id: i64, name: &str) -> Result<(), String> {
        let resp = self
            .client
            .patch(session_url(&self.base_url, session_id))
            .json(&serde_json::json!({ "name": name }))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(format!("HTTP {}", resp.status()))
        }
    }

    async fn delete_session(&self, session_id: i64) -> Result<(), String> {
        let resp = self
            .client
            .delete(session_url(&self.base_url, session_id))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(format!("HTTP {}", resp.status()))
        }
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
            .patch(active_session_url(&self.base_url, book_url))
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

    async fn get_server_settings(&self) -> Result<serde_json::Value, String> {
        let resp = self
            .client
            .get(format!("{}/api/settings", self.base_url))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if resp.status().is_success() {
            resp.json().await.map_err(|e| e.to_string())
        } else {
            Err(format!("HTTP {}", resp.status()))
        }
    }

    async fn update_server_settings(&self, settings: &serde_json::Value) -> Result<(), String> {
        let resp = self
            .client
            .patch(format!("{}/api/settings", self.base_url))
            .json(settings)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(format!("HTTP {}", resp.status()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = "http://127.0.0.1:8099";

    #[test]
    fn test_encode_segment_encodes_url_in_path() {
        assert_eq!(
            encode_segment("https://example.com/book?x=1&y=2"),
            "https%3A%2F%2Fexample.com%2Fbook%3Fx%3D1%26y%3D2"
        );
    }

    #[test]
    fn test_encode_segment_keeps_unreserved_chars() {
        assert_eq!(encode_segment("a-b_c.d~e"), "a-b_c.d~e");
    }

    #[test]
    fn test_get_book_url() {
        assert_eq!(
            book_url(BASE, "https://example.com/book"),
            "http://127.0.0.1:8099/api/books/https%3A%2F%2Fexample.com%2Fbook"
        );
    }

    #[test]
    fn test_delete_book_url() {
        assert_eq!(
            book_url(BASE, "http://example.com/book"),
            "http://127.0.0.1:8099/api/books/http%3A%2F%2Fexample.com%2Fbook"
        );
    }

    #[test]
    fn test_update_status_url() {
        assert_eq!(
            status_url(BASE, "https://example.com/book"),
            "http://127.0.0.1:8099/api/books/https%3A%2F%2Fexample.com%2Fbook/status"
        );
    }

    #[test]
    fn test_list_sessions_url() {
        assert_eq!(
            sessions_url(BASE, "https://example.com/book"),
            "http://127.0.0.1:8099/api/books/https%3A%2F%2Fexample.com%2Fbook/sessions"
        );
    }

    #[test]
    fn test_create_session_url() {
        assert_eq!(
            sessions_url(BASE, "https://example.com/book"),
            "http://127.0.0.1:8099/api/books/https%3A%2F%2Fexample.com%2Fbook/sessions"
        );
    }

    #[test]
    fn test_set_active_session_url() {
        assert_eq!(
            active_session_url(BASE, "https://example.com/book"),
            "http://127.0.0.1:8099/api/books/https%3A%2F%2Fexample.com%2Fbook/active-session"
        );
    }

    #[test]
    fn test_session_url() {
        assert_eq!(
            session_url(BASE, 42),
            "http://127.0.0.1:8099/api/sessions/42"
        );
    }

    #[test]
    fn test_new_trims_trailing_slash() {
        let api = RemoteApi::new("test".into(), "http://host/".into());
        assert_eq!(api.base_url, "http://host");
        let api = RemoteApi::new("test".into(), "http://host///".into());
        assert_eq!(api.base_url, "http://host");
        let api = RemoteApi::new("test".into(), "http://host".into());
        assert_eq!(api.base_url, "http://host");
    }
}
