use std::sync::Arc;

use axum::{Json, extract::State, http::StatusCode};

use crate::state::AppState;

pub async fn list_books(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<scylla_core::types::Book>>, StatusCode> {
    let db = state.db.lock().await;
    db.load_books().map(Json).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

pub async fn get_book(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(url): axum::extract::Path<String>,
) -> Result<Json<scylla_core::types::Book>, StatusCode> {
    let db = state.db.lock().await;
    let books = db.load_books().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    books.into_iter().find(|b| b.url == url)
        .map(Json).ok_or(StatusCode::NOT_FOUND)
}

pub async fn create_book(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<serde_json::Value>,
) -> StatusCode {
    let url = payload["url"].as_str().unwrap_or("");
    if url.is_empty() {
        return StatusCode::BAD_REQUEST;
    }
    let _ = state.cmd_tx.send(scylla_core::messenger::AppCommand::Scrape(url.to_string()));
    StatusCode::ACCEPTED
}

pub async fn delete_book(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(url): axum::extract::Path<String>,
) -> StatusCode {
    let db = state.db.lock().await;
    db.delete_book(&url).map(|_| StatusCode::NO_CONTENT)
        .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
}

pub async fn update_status(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(url): axum::extract::Path<String>,
    Json(payload): Json<serde_json::Value>,
) -> StatusCode {
    let status_str = payload["status"].as_str().unwrap_or("Reading");
    let status = match status_str {
        "Paused" => scylla_core::types::BookStatus::Paused,
        "Dropped" => scylla_core::types::BookStatus::Dropped,
        "Completed" => scylla_core::types::BookStatus::Completed,
        _ => scylla_core::types::BookStatus::Reading,
    };
    let db = state.db.lock().await;
    db.update_status(&url, &status).map(|_| StatusCode::OK)
        .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
}
