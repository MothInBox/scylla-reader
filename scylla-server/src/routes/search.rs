use std::sync::Arc;

use axum::{Json, extract::State, http::StatusCode};
use serde_json::json;

use crate::state::AppState;

/// POST /api/search — body `{"query": "...", "mode": "book"|"chapter", "limit": N?}`.
///
/// Embeds the query and returns the top matching books (by aggregate embedding)
/// or chapters (grouped by book).
pub async fn search(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let query = payload.get("query").and_then(|v| v.as_str()).unwrap_or("");
    if query.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "query is required" })),
        ));
    }
    let mode = payload
        .get("mode")
        .and_then(|v| v.as_str())
        .unwrap_or("book");
    if mode != "book" && mode != "chapter" {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "mode must be 'book' or 'chapter'" })),
        ));
    }
    let limit = payload
        .get("limit")
        .and_then(|v| v.as_u64())
        .map(|n| n.clamp(1, 50) as usize)
        .unwrap_or(10);

    if mode == "chapter" {
        search_chapters(&state, query, limit).await
    } else {
        search_books(&state, query, limit).await
    }
}

/// Chapter-mode search. An empty corpus (no rows in `chapter_embeddings`) is an
/// expected first-run state, not a server error: it returns a distinct
/// `no_embeddings` response. The corpus is checked before the embedder so an
/// offline first run still gets the hint instead of a generic 503.
async fn search_chapters(
    state: &AppState,
    query: &str,
    limit: usize,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let db = state.db.lock().await;
    let chapters = db.load_all_chapter_embeddings().map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "failed to load chapter embeddings" })),
        )
    })?;
    let total = db.chapter_count().map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "failed to count chapters" })),
        )
    })?;
    drop(db);

    if chapters.is_empty() {
        return Ok(Json(json!({
            "error": "no_embeddings",
            "embedded": 0,
            "total": total,
        })));
    }

    let query_vec = embed_query(state, query)?;
    let ranked = crate::embeddings::rank_chapters(&chapters, &query_vec, limit);
    let results: Vec<serde_json::Value> = ranked
        .into_iter()
        .map(
            |(book_url, book_title, chapter_url, chapter_title, chapter_idx, genres, score)| {
                json!({
                    "book_url": book_url,
                    "book_title": book_title,
                    "chapter_url": chapter_url,
                    "chapter_idx": chapter_idx,
                    "chapter_title": chapter_title,
                    "score": score,
                    "genres": genres.unwrap_or_default(),
                })
            },
        )
        .collect();
    Ok(Json(json!({ "mode": "chapter", "results": results })))
}

/// Book-mode search (unchanged behavior).
async fn search_books(
    state: &AppState,
    query: &str,
    limit: usize,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let query_vec = embed_query(state, query)?;
    let db = state.db.lock().await;
    let books = db.load_all_book_embeddings().map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "failed to load book embeddings" })),
        )
    })?;
    drop(db);

    let ranked = crate::embeddings::rank_books(&books, &query_vec, limit);
    let results: Vec<serde_json::Value> = ranked
        .into_iter()
        .map(|(book_url, score, genres)| {
            json!({
                "book_url": book_url,
                "score": score,
                "genres": genres.unwrap_or_default(),
            })
        })
        .collect();
    Ok(Json(json!({ "mode": "book", "results": results })))
}

/// Embeds the query with the shared embedder, returning a 503 when the model
/// isn't available (offline first run / post-failure cooldown).
fn embed_query(
    state: &AppState,
    query: &str,
) -> Result<Vec<f32>, (StatusCode, Json<serde_json::Value>)> {
    let Some(embed) = state.embedder.get() else {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "error": "embedding model not loaded (offline first run)" })),
        ));
    };
    embed(&[query])
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "failed to embed query" })),
            )
        })
        .map(|mut v| v.remove(0))
}
