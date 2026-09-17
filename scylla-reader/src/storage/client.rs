//! A shared Tokio runtime so the TUI's main thread can drive async backend
//! calls from synchronous code, plus small HTTP helpers for the server-owned
//! job API.

use crate::event_types::ChapterHit;
use crate::state::AppState;
use crate::storage::manager::LibraryManager;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde::Deserialize;
use std::future::Future;
use std::sync::OnceLock;
use tokio::runtime::Runtime;

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

fn runtime() -> &'static Runtime {
    static RT: OnceLock<Runtime> = OnceLock::new();
    RT.get_or_init(|| Runtime::new().expect("failed to create tokio runtime"))
}

pub fn block_on<F: Future>(fut: F) -> F::Output {
    runtime().block_on(fut)
}

/// Base URL of the primary backend, or the local default when none is
/// configured.
pub fn api_base(state: &AppState) -> String {
    api_base_for(&state.lib.manager)
}

/// Base URL of the primary backend, or the local default when none is
/// configured.
pub fn api_base_for(manager: &LibraryManager) -> String {
    manager
        .primary_backend()
        .and_then(|b| b.url())
        .unwrap_or_else(|| "http://127.0.0.1:8080".to_string())
}

/// POST /api/jobs/enqueue — `kind` is "Scrape" | "FetchChapter" | "FetchCover".
pub async fn enqueue_job(
    base: &str,
    kind: &str,
    url: &str,
    chapter_idx: Option<usize>,
) -> Result<(), String> {
    let mut body = serde_json::json!({ "kind": kind, "url": url });
    if let Some(idx) = chapter_idx {
        body["chapter_idx"] = serde_json::json!(idx);
    }
    post_json(&format!("{}/api/jobs/enqueue", base), &body).await
}

/// POST /api/jobs/enqueue — kind "EmbedBatch" with a chapters array
/// `(url, idx, title)`. The server runs ONE job that scrapes and embeds each
/// chapter, emitting `JobDetailChanged` events with per-chapter status.
pub async fn enqueue_embed_batch(
    base: &str,
    chapters: &[(String, usize, String)],
) -> Result<(), String> {
    let chapters_json: Vec<serde_json::Value> = chapters
        .iter()
        .map(|(url, idx, title)| serde_json::json!({ "url": url, "idx": idx, "title": title }))
        .collect();
    let body = serde_json::json!({ "kind": "EmbedBatch", "chapters": chapters_json });
    post_json(&format!("{}/api/jobs/enqueue", base), &body).await
}

/// PATCH /api/settings — set the server's autoembed flag.
pub async fn update_autoembed(base: &str, autoembed: bool) -> Result<(), String> {
    let client = reqwest::Client::new();
    let resp = client
        .patch(format!("{}/api/settings", base))
        .json(&serde_json::json!({ "autoembed": autoembed }))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if resp.status().is_success() {
        Ok(())
    } else {
        Err(format!("HTTP {}", resp.status()))
    }
}

/// POST /api/books — scrape trigger.
pub async fn post_book(base: &str, url: &str) -> Result<(), String> {
    post_json(
        &format!("{}/api/books", base),
        &serde_json::json!({ "url": url }),
    )
    .await
}

/// POST /api/plugins/install.
pub async fn install_plugin(base: &str, repo_url: &str) -> Result<(), String> {
    post_json(
        &format!("{}/api/plugins/install", base),
        &serde_json::json!({ "repo_url": repo_url }),
    )
    .await
}

/// Generic POST to `/api/jobs/{path}` (cancel, retry, flush, workers, ...).
pub async fn job_command(base: &str, path: &str, body: serde_json::Value) -> Result<(), String> {
    post_json(&format!("{}/api/jobs/{}", base, path), &body).await
}

/// POST /api/search — chapter-mode AI search. Returns the flat chapter hits.
///
/// The TUI only uses chapter mode; book mode is rejected up front. The client
/// uses a generous 60s timeout because the first search legitimately includes
/// the ~91MB embedding-model download.
pub async fn search(
    base: &str,
    query: &str,
    mode: &str,
    limit: usize,
) -> Result<Vec<ChapterHit>, String> {
    if mode != "chapter" {
        return Err("book mode not supported by the TUI client".to_string());
    }
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .expect("failed to build http client");
    let resp = client
        .post(format!("{}/api/search", base))
        .json(&serde_json::json!({ "query": query, "mode": mode, "limit": limit }))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let json: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    Ok(parse_search_response(&json))
}

/// Embedding progress for a book, from `GET /api/books/:url/embedding-status`.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct EmbeddingStatus {
    pub embedded_chapters: usize,
    pub total_chapters: usize,
    pub aggregate: bool,
    #[serde(default)]
    pub genres: Vec<String>,
    #[serde(default)]
    pub embedded_chapter_urls: Vec<String>,
}

/// GET /api/books/:url/embedding-status — how many of the book's chapters have
/// embeddings, whether the aggregate exists, and the genres. 404 (book not
/// found / not embedded yet) maps to `Err("not embedded yet")`.
pub async fn embedding_status(base: &str, book_url: &str) -> Result<EmbeddingStatus, String> {
    let client = reqwest::Client::new();
    let url = format!(
        "{}/api/books/{}/embedding-status",
        base,
        encode_segment(book_url)
    );
    let resp = client.get(&url).send().await.map_err(|e| e.to_string())?;
    if resp.status().as_u16() == 404 {
        return Err("not embedded yet".to_string());
    }
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    resp.json().await.map_err(|e| e.to_string())
}

/// Parse the `results` array of a search response into `ChapterHit`s, logging
/// any malformed entries.
fn parse_search_response(json: &serde_json::Value) -> Vec<ChapterHit> {
    let Some(results) = json.get("results").and_then(|r| r.as_array()) else {
        return Vec::new();
    };
    let mut dropped = 0;
    let hits: Vec<ChapterHit> = results
        .iter()
        .filter_map(|v| match serde_json::from_value(v.clone()) {
            Ok(hit) => Some(hit),
            Err(_) => {
                dropped += 1;
                None
            }
        })
        .collect();
    if dropped > 0 {
        crate::settings::log(
            crate::settings::LogLevel::Error,
            "SEARCH",
            &format!("Dropped {} malformed search hit(s)", dropped),
        );
    }
    hits
}

async fn post_json(url: &str, body: &serde_json::Value) -> Result<(), String> {
    let client = reqwest::Client::new();
    let resp = client
        .post(url)
        .json(body)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if resp.status().is_success() || resp.status().as_u16() == 202 {
        Ok(())
    } else {
        Err(format!("HTTP {}", resp.status()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::Library;
    use crate::test_helpers::test_state_with_backend;

    #[test]
    fn test_api_base_defaults_to_localhost() {
        let state = AppState::from_parts(Library::new());
        assert_eq!(api_base(&state), "http://127.0.0.1:8080");
    }

    #[test]
    fn test_api_base_uses_primary_backend_url() {
        let state =
            test_state_with_backend(Box::new(crate::test_helpers::MockBackend::new("mock")));
        assert_eq!(api_base(&state), "http://mock");
    }

    #[test]
    fn test_parse_search_response_flat_hits() {
        let json = serde_json::json!({
            "mode": "chapter",
            "results": [
                {"book_url":"u1","book_title":"B1","chapter_url":"c1","chapter_idx":0,"chapter_title":"C1","score":0.9,"genres":["Fantasy"]},
                {"book_url":"u2","book_title":"B2","chapter_url":"c2","chapter_idx":1,"chapter_title":"C2","score":0.5}
            ]
        });
        let hits = parse_search_response(&json);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].book_title, "B1");
        assert_eq!(hits[0].score, 0.9);
        assert_eq!(hits[0].genres, vec!["Fantasy"]);
        assert!(hits[1].genres.is_empty());
    }

    #[test]
    fn test_parse_search_response_missing_results_is_empty() {
        let json = serde_json::json!({ "mode": "chapter" });
        assert!(parse_search_response(&json).is_empty());
    }

    #[test]
    fn test_parse_search_response_drops_malformed() {
        let json = serde_json::json!({
            "results": [
                {"book_url":"u1","book_title":"B1","chapter_url":"c1","chapter_idx":0,"chapter_title":"C1","score":0.9},
                {"book_url":"u2"}  // missing required fields
            ]
        });
        let hits = parse_search_response(&json);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].book_title, "B1");
    }

    #[test]
    fn test_search_rejects_book_mode() {
        // The guard returns before any HTTP request is made.
        let result = block_on(search("http://127.0.0.1:1", "q", "book", 10));
        assert_eq!(
            result.err().as_deref(),
            Some("book mode not supported by the TUI client")
        );
    }

    #[test]
    fn test_embedding_status_deserializes_from_response_json() {
        let json = r#"{"embedded_chapters":12,"total_chapters":40,"aggregate":true,"genres":["Fantasy","LitRPG"],"embedded_chapter_urls":["http://example.com/book/ch1","http://example.com/book/ch2"]}"#;
        let status: EmbeddingStatus = serde_json::from_str(json).unwrap();
        assert_eq!(status.embedded_chapters, 12);
        assert_eq!(status.total_chapters, 40);
        assert!(status.aggregate);
        assert_eq!(status.genres, vec!["Fantasy", "LitRPG"]);
        assert_eq!(
            status.embedded_chapter_urls,
            vec![
                "http://example.com/book/ch1".to_string(),
                "http://example.com/book/ch2".to_string()
            ]
        );
    }

    #[test]
    fn test_embedding_status_genres_default_when_missing() {
        let json = r#"{"embedded_chapters":0,"total_chapters":5,"aggregate":false}"#;
        let status: EmbeddingStatus = serde_json::from_str(json).unwrap();
        assert_eq!(status.embedded_chapters, 0);
        assert_eq!(status.total_chapters, 5);
        assert!(!status.aggregate);
        assert!(status.genres.is_empty());
        assert!(status.embedded_chapter_urls.is_empty());
    }

    #[test]
    fn test_embedding_status_url_encodes_book_url() {
        let url = format!(
            "{}/api/books/{}/embedding-status",
            "http://127.0.0.1:8099",
            encode_segment("https://example.com/book")
        );
        assert_eq!(
            url,
            "http://127.0.0.1:8099/api/books/https%3A%2F%2Fexample.com%2Fbook/embedding-status"
        );
    }
}
