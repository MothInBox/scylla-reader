use std::sync::Arc;

use axum::{Json, extract::State, http::StatusCode};

use crate::state::AppState;

pub async fn list_sessions(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(book_url): axum::extract::Path<String>,
) -> Result<Json<Vec<scylla_core::types::Session>>, StatusCode> {
    let db = state.db.lock().await;
    db.load_sessions_for_book(&book_url)
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

pub async fn create_session(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(book_url): axum::extract::Path<String>,
    Json(payload): Json<serde_json::Value>,
) -> Result<(StatusCode, Json<scylla_core::types::Session>), StatusCode> {
    let name = payload["name"].as_str().unwrap_or("");
    let total = payload["total"].as_u64().unwrap_or(0) as u32;
    let db = state.db.lock().await;
    let session = db
        .create_session(&book_url, name, total)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok((StatusCode::CREATED, Json(session)))
}

pub async fn rename_session(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(session_id): axum::extract::Path<i64>,
    Json(payload): Json<serde_json::Value>,
) -> StatusCode {
    let name = payload["name"].as_str().unwrap_or("");
    let db = state.db.lock().await;
    match db.rename_session(session_id, name) {
        Ok(0) => StatusCode::NOT_FOUND,
        Ok(_) => StatusCode::OK,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

pub async fn delete_session(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(session_id): axum::extract::Path<i64>,
) -> StatusCode {
    let db = state.db.lock().await;
    match db.delete_session(session_id) {
        Ok(0) => StatusCode::NOT_FOUND,
        Ok(_) => StatusCode::NO_CONTENT,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

pub async fn update_progress(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(session_id): axum::extract::Path<i64>,
    Json(payload): Json<serde_json::Value>,
) -> StatusCode {
    let current = payload["current"].as_u64().unwrap_or(0) as u32;
    let db = state.db.lock().await;
    db.update_session_progress(session_id, current)
        .map(|_| StatusCode::OK)
        .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
}

pub async fn set_active_session(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(book_url): axum::extract::Path<String>,
    Json(payload): Json<serde_json::Value>,
) -> StatusCode {
    let session_id = payload["session_id"].as_i64();
    let db = state.db.lock().await;
    db.set_active_session(&book_url, session_id)
        .map(|_| StatusCode::OK)
        .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
}
