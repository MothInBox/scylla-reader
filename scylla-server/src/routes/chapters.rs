use std::sync::Arc;

use axum::{Json, extract::State, http::StatusCode};
use serde_json::json;

use crate::state::AppState;

/// GET /api/books/:book_url/chapters/:chapter_url/content — direct chapter
/// fetch for web clients.
///
/// Note: the TUI does NOT use this route — it enqueues a `FetchChapter` job
/// (`POST /api/jobs/enqueue`) so the fetch runs through the worker (rate
/// limiting, job tracking, SSE events). This route bypasses the worker and
/// performs no embedding.
pub async fn get_chapter_content(
    State(state): State<Arc<AppState>>,
    axum::extract::Path((_book_url, chapter_url)): axum::extract::Path<(String, String)>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let registry = state.registry.clone();
    let handle = tokio::runtime::Handle::current();
    let result = tokio::task::spawn_blocking(move || {
        let reg = registry
            .lock()
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        handle
            .block_on(reg.scrape_chapter(&chapter_url))
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
    })
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let (title, content) = result?;
    Ok(Json(json!({ "title": title, "content": content })))
}
