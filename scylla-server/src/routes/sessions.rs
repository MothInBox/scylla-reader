use std::sync::Arc;

use axum::{Json, extract::State, http::StatusCode};

use crate::state::AppState;

pub async fn list_sessions(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(book_url): axum::extract::Path<String>,
) -> Result<Json<Vec<scylla_core::types::Session>>, StatusCode> {
    let db = state.db.lock().await;
    db.load_sessions_for_book(&book_url).map(Json).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

pub async fn update_progress(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(session_id): axum::extract::Path<i64>,
    Json(payload): Json<serde_json::Value>,
) -> StatusCode {
    let current = payload["current"].as_u64().unwrap_or(0) as u32;
    let db = state.db.lock().await;
    db.update_session_progress(session_id, current).map(|_| StatusCode::OK)
        .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
}

pub async fn set_active_session(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(book_url): axum::extract::Path<String>,
    Json(payload): Json<serde_json::Value>,
) -> StatusCode {
    let session_id = payload["session_id"].as_i64();
    let db = state.db.lock().await;
    db.set_active_session(&book_url, session_id).map(|_| StatusCode::OK)
        .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
}
