use std::collections::HashMap;
use std::sync::Arc;

use axum::{Json, extract::State, http::StatusCode};
use serde_json::json;

use crate::state::AppState;

/// Search route error: an HTTP status plus a JSON error body.
type SearchError = (StatusCode, Json<serde_json::Value>);

/// POST /api/search — body
/// `{"query": "...", "mode": "book"|"chapter", "limit": N?, "book_url": "..."?}`.
///
/// Embeds the query and returns the top matching books (by aggregate embedding)
/// or chapters (grouped by book). `book_url` restricts chapter mode to one
/// book's chapters (the per-book drill-down window); it is ignored in book mode.
///
/// Score contract: every hit's `score` is a **display-only** normalized 0–100
/// value (fixed mapping from the cosine range, see `embeddings::normalize_score`),
/// not a raw similarity measure. Scores are comparable across result sets but
/// must not be interpreted as probabilities or raw cosines.
pub async fn search(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, SearchError> {
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
    let book_url = payload
        .get("book_url")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    if mode == "chapter" {
        search_chapters(&state, query, limit, book_url.as_deref()).await
    } else {
        search_books(&state, query, limit).await
    }
}

/// Chapter-mode search. An empty corpus (no rows in `chapter_embeddings`) is an
/// expected first-run state, not a server error: it returns a distinct
/// `no_embeddings` response. The corpus is checked before the embedder so an
/// offline first run still gets the hint instead of a generic 503.
///
/// With `book_url` set, only that book's chapters are searched (drill-down).
/// The per-book diversity cap is a no-op for single-book inputs, so the
/// drill-down gets its own top-`limit` unfiltered. A book with no embeddings
/// returns empty results — the `no_embeddings` hint is library-level only.
async fn search_chapters(
    state: &AppState,
    query: &str,
    limit: usize,
    book_url: Option<&str>,
) -> Result<Json<serde_json::Value>, SearchError> {
    let db = state.db.lock().await;
    let chapters = match book_url {
        Some(url) => db.load_chapter_hits_for_book(url).map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "failed to load chapter embeddings" })),
            )
        })?,
        None => db.load_all_chapter_embeddings().map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "failed to load chapter embeddings" })),
            )
        })?,
    };
    let embedded = db.embedded_chapter_count().map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "failed to count chapter embeddings" })),
        )
    })?;
    let total = db.chapter_count().map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "failed to count chapters" })),
        )
    })?;
    drop(db);

    if book_url.is_none() && chapters.is_empty() {
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
    Ok(Json(json!({
        "mode": "chapter",
        "embedded": embedded,
        "total": total,
        "results": results,
    })))
}

/// Book-mode search. Each book hit carries its top-3 chapter hits inline so the
/// TUI drill-down is a local filter of already-received data.
async fn search_books(
    state: &AppState,
    query: &str,
    limit: usize,
) -> Result<Json<serde_json::Value>, SearchError> {
    let query_vec = embed_query(state, query)?;
    let db = state.db.lock().await;
    let books = db.load_all_book_embeddings_with_titles().map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "failed to load book embeddings" })),
        )
    })?;
    let chapters = db.load_all_chapter_embeddings().map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "failed to load chapter embeddings" })),
        )
    })?;
    let embedded = db.embedded_chapter_count().map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "failed to count chapter embeddings" })),
        )
    })?;
    let total = db.chapter_count().map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "failed to count chapters" })),
        )
    })?;
    drop(db);

    // Rank books by aggregate embedding.
    let aggregates: Vec<crate::embeddings::BookAggregate> = books
        .iter()
        .map(|(url, _, agg, genres)| (url.clone(), agg.clone(), genres.clone()))
        .collect();
    let ranked_books = crate::embeddings::rank_books(&aggregates, &query_vec, limit);

    // Rank every chapter once (no truncation) so each book's best 3 are
    // available, then group them per book.
    let ranked_chapters = crate::embeddings::rank_chapters(&chapters, &query_vec, chapters.len());
    let by_book = crate::embeddings::top_chapters_per_book(ranked_chapters, 3);

    let titles: HashMap<String, String> = books
        .iter()
        .map(|(url, title, _, _)| (url.clone(), title.clone()))
        .collect();

    let hits: Vec<serde_json::Value> = ranked_books
        .into_iter()
        .map(|(book_url, score, _genres)| {
            let chapters = by_book
                .get(&book_url)
                .map(|group| {
                    group
                        .iter()
                        .map(
                            |(_, _, chapter_url, chapter_title, chapter_idx, _, score)| {
                                json!({
                                    "url": chapter_url,
                                    "chapter_idx": chapter_idx,
                                    "title": chapter_title,
                                    "score": score,
                                })
                            },
                        )
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            json!({
                "book_url": book_url,
                "title": titles.get(&book_url).cloned().unwrap_or_default(),
                "score": crate::embeddings::normalize_score(score),
                "chapters": chapters,
            })
        })
        .collect();
    Ok(Json(json!({
        "mode": "book",
        "embedded": embedded,
        "total": total,
        "hits": hits,
    })))
}

/// Embeds the query with the shared embedder, returning a 503 when the model
/// isn't available (offline first run / post-failure cooldown).
fn embed_query(state: &AppState, query: &str) -> Result<Vec<f32>, SearchError> {
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
