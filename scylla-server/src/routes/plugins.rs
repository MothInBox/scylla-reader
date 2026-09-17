use std::sync::Arc;

use axum::{Json, extract::State, http::StatusCode};

use crate::state::AppState;

/// POST /api/plugins/install — body `{"repo_url": "..."}`.
pub async fn install_plugin(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<serde_json::Value>,
) -> StatusCode {
    let Some(repo_url) = payload.get("repo_url").and_then(|v| v.as_str()) else {
        return StatusCode::BAD_REQUEST;
    };
    if repo_url.is_empty() {
        return StatusCode::BAD_REQUEST;
    }
    let _ = state
        .cmd_tx
        .send(scylla_core::messenger::AppCommand::InstallPlugin(
            repo_url.to_string(),
        ));
    StatusCode::ACCEPTED
}
