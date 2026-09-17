use std::sync::Arc;

use axum::{Json, extract::State, http::StatusCode};
use serde_json::json;

use crate::state::AppState;

pub async fn get_settings(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let max_workers = *state.max_workers.lock().unwrap();
    let rate_limit = *state.rate_limit.lock().unwrap();
    Json(json!({
        "port": 8080,
        "max_workers": max_workers,
        "rate_limit": rate_limit,
        "plugins": [],
    }))
}

pub async fn update_settings(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<serde_json::Value>,
) -> StatusCode {
    if let Some(n) = payload.get("max_workers").and_then(|v| v.as_u64()) {
        *state.max_workers.lock().unwrap() = n as u8;
        let _ = state
            .cmd_tx
            .send(scylla_core::messenger::AppCommand::SetMaxWorkers(n as u8));
    }
    if let Some(s) = payload.get("rate_limit").and_then(|v| v.as_u64()) {
        *state.rate_limit.lock().unwrap() = s;
        let _ = state
            .cmd_tx
            .send(scylla_core::messenger::AppCommand::SetRateLimit(s));
    }
    StatusCode::OK
}
