use std::sync::Arc;

use axum::{Json, extract::State, http::StatusCode};
use serde_json::json;

use crate::state::AppState;

/// Embeddings loaded under the DB lock, ranked after it is released so a large
/// library doesn't block other HTTP handlers during scoring.
enum Loaded {
    Books(Vec<crate::embeddings::BookAggregate>),
    Chapters(Vec<crate::embeddings::ChapterHit>),
}

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

    let Some(embed) = state.embedder.get() else {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "error": "embedding model not loaded (offline first run)" })),
        ));
    };
    let query_vec = embed(query).map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "failed to embed query" })),
        )
    })?;

    // Load the embeddings under the lock, then drop it before ranking so a
    // large library doesn't block other HTTP handlers during scoring.
    let db = state.db.lock().await;
    let loaded = match mode {
        "book" => {
            let books = db.load_all_book_embeddings().map_err(|_| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({ "error": "failed to load book embeddings" })),
                )
            })?;
            Loaded::Books(books)
        }
        _ => {
            let chapters = db.load_all_chapter_embeddings().map_err(|_| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({ "error": "failed to load chapter embeddings" })),
                )
            })?;
            Loaded::Chapters(chapters)
        }
    };
    drop(db);

    match loaded {
        Loaded::Books(books) => {
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
        Loaded::Chapters(chapters) => {
            let ranked = crate::embeddings::rank_chapters(&chapters, &query_vec, limit);
            let results: Vec<serde_json::Value> = ranked
                .into_iter()
                .map(
                    |(
                        book_url,
                        book_title,
                        chapter_url,
                        chapter_title,
                        chapter_idx,
                        genres,
                        score,
                    )| {
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
    }
}
