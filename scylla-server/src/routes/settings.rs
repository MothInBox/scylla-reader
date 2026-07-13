use std::sync::Arc;

use axum::{Json, extract::State, http::StatusCode};
use serde_json::json;

use crate::state::AppState;

pub async fn get_settings(
    State(_state): State<Arc<AppState>>,
) -> Json<serde_json::Value> {
    Json(json!({
        "port": 8080,
        "max_workers": 4,
        "rate_limit": 2,
        "plugins": [],
    }))
}

pub async fn update_settings(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<serde_json::Value>,
) -> StatusCode {
    if let Some(n) = payload.get("max_workers").and_then(|v| v.as_u64()) {
        let _ = state.cmd_tx.send(scylla_core::messenger::AppCommand::SetMaxWorkers(n as u8));
    }
    if let Some(s) = payload.get("rate_limit").and_then(|v| v.as_u64()) {
        let _ = state.cmd_tx.send(scylla_core::messenger::AppCommand::SetRateLimit(s));
    }
    StatusCode::OK
}
