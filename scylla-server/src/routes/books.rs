use std::sync::Arc;

use axum::{Json, extract::State, http::StatusCode};
use serde_json::json;

use crate::state::AppState;

/// Serializes a book with its genres attached (the genre facet's data source on
/// the library listing path). `scylla_core::types::Book` has no genres field,
/// so the route enriches the JSON — the TUI's own `Book` model ignores unknown
/// fields, so this is wire-compatible.
fn book_json(book: &scylla_core::types::Book, genres: &[String]) -> serde_json::Value {
    let mut v = serde_json::to_value(book).unwrap_or(serde_json::Value::Null);
    v["genres"] = serde_json::to_value(genres).unwrap_or(serde_json::Value::Null);
    v
}

pub async fn list_books(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<serde_json::Value>>, StatusCode> {
    let db = state.db.lock().await;
    let books = db
        .load_books()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let genres = db
        .load_book_genres()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let out = books
        .iter()
        .map(|b| book_json(b, genres.get(&b.url).map(Vec::as_slice).unwrap_or(&[])))
        .collect();
    Ok(Json(out))
}

pub async fn get_book(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(url): axum::extract::Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let db = state.db.lock().await;
    let books = db
        .load_books()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let genres = db
        .load_book_genres()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    books
        .iter()
        .find(|b| b.url == url)
        .map(|b| {
            Json(book_json(
                b,
                genres.get(&b.url).map(Vec::as_slice).unwrap_or(&[]),
            ))
        })
        .ok_or(StatusCode::NOT_FOUND)
}

pub async fn upsert_book(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(url): axum::extract::Path<String>,
    Json(payload): Json<serde_json::Value>,
) -> StatusCode {
    let book: scylla_core::types::Book = match serde_json::from_value(payload) {
        Ok(book) => book,
        Err(_) => return StatusCode::BAD_REQUEST,
    };
    if book.url != url {
        return StatusCode::BAD_REQUEST;
    }
    let db = state.db.lock().await;
    db.upsert_book(&book)
        .map(|_| StatusCode::OK)
        .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
}

pub async fn create_book(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<serde_json::Value>,
) -> StatusCode {
    let url = payload["url"].as_str().unwrap_or("");
    if url.is_empty() {
        return StatusCode::BAD_REQUEST;
    }
    let _ = state
        .cmd_tx
        .send(scylla_core::messenger::AppCommand::Scrape(url.to_string()));
    StatusCode::ACCEPTED
}

pub async fn delete_book(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(url): axum::extract::Path<String>,
) -> StatusCode {
    let db = state.db.lock().await;
    db.delete_book(&url)
        .map(|_| StatusCode::NO_CONTENT)
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
    db.update_status(&url, &status)
        .map(|_| StatusCode::OK)
        .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
}

/// POST /api/books/:url/crawl — enqueues a FetchChapter job for every stored
/// chapter of the book (the worker + embedding queue handle the rest).
pub async fn crawl_book(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(url): axum::extract::Path<String>,
) -> StatusCode {
    let chapters = {
        let db = state.db.lock().await;
        match db.load_chapters(&url) {
            Ok(c) => c,
            Err(_) => return StatusCode::INTERNAL_SERVER_ERROR,
        }
    };
    if chapters.is_empty() {
        return StatusCode::NOT_FOUND;
    }
    for ch in &chapters {
        let _ = state
            .cmd_tx
            .send(scylla_core::messenger::AppCommand::FetchChapter(
                ch.url.clone(),
                ch.order as usize,
            ));
    }
    StatusCode::ACCEPTED
}

/// GET /api/books/:url/embedding-status — how many of the book's chapters have
/// embeddings (and which), whether the aggregate exists, and the genres.
pub async fn embedding_status(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(url): axum::extract::Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let db = state.db.lock().await;
    let status = db
        .embedding_status(&url)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let Some((embedded_chapters, total_chapters, has_aggregate, genres, embedded_chapter_urls)) =
        status
    else {
        return Err(StatusCode::NOT_FOUND);
    };
    Ok(Json(json!({
        "embedded_chapters": embedded_chapters,
        "total_chapters": total_chapters,
        "aggregate": has_aggregate,
        "genres": genres.unwrap_or_default(),
        "embedded_chapter_urls": embedded_chapter_urls,
    })))
}
