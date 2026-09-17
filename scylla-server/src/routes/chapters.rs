use std::sync::Arc;

use axum::{Json, extract::State, http::StatusCode};
use serde_json::json;

use crate::state::AppState;

pub async fn get_chapter_content(
    State(state): State<Arc<AppState>>,
    axum::extract::Path((_book_url, chapter_url)): axum::extract::Path<(String, String)>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let reg = state
        .registry
        .lock()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let rt = tokio::runtime::Handle::current();
    let (title, content) = rt
        .block_on(reg.scrape_chapter(&chapter_url))
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(json!({ "title": title, "content": content })))
}
