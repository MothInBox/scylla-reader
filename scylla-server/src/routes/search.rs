use std::collections::HashMap;
use std::sync::Arc;

use axum::{Json, extract::State, http::StatusCode};
use serde_json::json;

use crate::embeddings::{RankedBook, RankedChapterHit};
use crate::state::AppState;

/// Search route error: an HTTP status plus a JSON error body.
type SearchError = (StatusCode, Json<serde_json::Value>);

/// Runs CPU-bound search work (embed + rank + rerank) on the blocking pool so
/// the tokio worker never stalls. A panicked task becomes a 500.
async fn run_blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, SearchError> + Send + 'static,
) -> Result<T, SearchError> {
    tokio::task::spawn_blocking(f).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("search task failed: {e}") })),
        )
    })?
}

/// How many candidates the bi-encoder returns before the cross-encoder rerank
/// (and before truncation to the requested `limit`).
const RERANK_CANDIDATES: usize = 50;

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
/// must not be interpreted as probabilities or raw cosines. When the
/// cross-encoder reranker is available, the *order* is refined by relevance
/// scores but the displayed values stay the bi-encoder's normalized cosines.
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
        search_chapters(state, query, limit, book_url.as_deref()).await
    } else {
        search_books(state, query, limit).await
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
///
/// Wire shape (pinned): `{"mode":"chapter","embedded":X,"total":Y,"hits":[
/// {"url","chapter_idx","title","score"}]}` — see the shared fixture in the
/// server and TUI tests.
async fn search_chapters(
    state: Arc<AppState>,
    query: &str,
    limit: usize,
    book_url: Option<&str>,
) -> Result<Json<serde_json::Value>, SearchError> {
    let db = state.db.lock().await;
    let chapters = match book_url {
        Some(url) => db.load_chunk_hits_for_book(url).map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "failed to load chapter embeddings" })),
            )
        })?,
        None => db.load_all_chunk_hits().map_err(|_| {
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

    // Embed + rank + rerank are CPU-bound (model forward passes) — run them on
    // the blocking pool so the tokio worker never stalls.
    let query = query.to_string();
    let query_for_snippet = query.clone();
    let ranked = run_blocking(move || {
        let query_vec = embed_query(&state, &query)?;
        Ok::<_, SearchError>(fuse_and_rerank(&state, &query, &chapters, &query_vec))
    })
    .await?;

    let hits: Vec<serde_json::Value> = ranked
        .into_iter()
        .take(limit)
        .map(
            |(_, _, chapter_url, chapter_title, chapter_idx, _, score, text)| {
                json!({
                    "url": chapter_url,
                    "chapter_idx": chapter_idx,
                    "title": chapter_title,
                    "score": score,
                    "snippet": make_snippet(&text, &query_for_snippet),
                })
            },
        )
        .collect();
    Ok(Json(json!({
        "mode": "chapter",
        "embedded": embedded,
        "total": total,
        "hits": hits,
    })))
}

/// Book-mode search. Each book hit carries its top-3 chapter hits inline so the
/// TUI drill-down is a local filter of already-received data.
///
/// An empty corpus (no chapter embeddings) is the same first-run state as
/// chapter mode: it returns the distinct `no_embeddings` response, checked
/// before the embedder so an offline first run gets the hint, not a 503.
async fn search_books(
    state: Arc<AppState>,
    query: &str,
    limit: usize,
) -> Result<Json<serde_json::Value>, SearchError> {
    let db = state.db.lock().await;
    let books = db.load_all_book_embeddings_with_titles().map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "failed to load book embeddings" })),
        )
    })?;
    let chapters = db.load_all_chunk_hits().map_err(|_| {
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

    if chapters.is_empty() {
        return Ok(Json(json!({
            "error": "no_embeddings",
            "embedded": 0,
            "total": total,
        })));
    }

    // Embed + rank + rerank are CPU-bound — run them on the blocking pool.
    let query = query.to_string();
    let query_for_snippet = query.clone();
    // Titles are needed after the closure; build them first so `books` (with
    // its ~4.6MB of aggregate embeddings at 3000 books) can be moved into the
    // closure and consumed without cloning.
    let titles: HashMap<String, String> = books
        .iter()
        .map(|(url, title, _, _)| (url.clone(), title.clone()))
        .collect();
    let (ranked_books, by_book) = run_blocking(move || {
        let query_vec = embed_query(&state, &query)?;

        // Rank books by aggregate embedding (top-50 candidates for rerank).
        let aggregates: Vec<crate::embeddings::BookAggregate> = books
            .into_iter()
            .map(|(url, _, agg, genres)| (url, agg, genres))
            .collect();
        let ranked_books =
            crate::embeddings::rank_books(&aggregates, &query_vec, RERANK_CANDIDATES);

        // Fuse the semantic + BM25 chapter lanes, rerank, then group per book
        // so each book's inline chapters come from the hybrid ranking.
        let fused = fuse_and_rerank(&state, &query, &chapters, &query_vec);
        let by_book = crate::embeddings::top_chapters_per_book(fused, 3);

        let ranked_books = rerank_books(&state, &query, ranked_books, &by_book);
        Ok::<_, SearchError>((ranked_books, by_book))
    })
    .await?;

    let hits: Vec<serde_json::Value> = ranked_books
        .into_iter()
        .take(limit)
        .map(|(book_url, score, genres)| {
            let chapters = by_book
                .get(&book_url)
                .map(|group| {
                    group
                        .iter()
                        .map(
                            |(_, _, chapter_url, chapter_title, chapter_idx, _, score, text)| {
                                json!({
                                    "url": chapter_url,
                                    "chapter_idx": chapter_idx,
                                    "title": chapter_title,
                                    "score": score,
                                    "snippet": make_snippet(text, &query_for_snippet),
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
                "genres": genres.unwrap_or_default(),
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
    embed(&[query], crate::embeddings::EmbedMode::Query)
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "failed to embed query" })),
            )
        })
        .map(|mut v| v.remove(0))
}

/// Builds a snippet for a chapter hit from its passage text (`RankedChapterHit`
/// index 7). For BM25-surfaced chapters that text is the BM25-best chunk, so
/// centering on the first query token present in it lands on the matched term
/// span; for semantic hits the same rule is a bonus (centers on a query term if
/// one happens to appear) and otherwise falls back to the start of the text.
fn make_snippet(text: &str, query: &str) -> String {
    // Lowercase the text once (not once per query token) and find the first
    // query token present in it.
    let lower = text.to_lowercase();
    let center = query
        .split_whitespace()
        .map(str::to_lowercase)
        .find(|t| lower.contains(t.as_str()));
    crate::snippet::snippet(text, center.as_deref(), crate::snippet::SNIPPET_LEN)
}

/// Cross-encoder rerank for chapter hits: scores each candidate's best chunk
/// text against the query and re-sorts. Falls back to the bi-encoder order when
/// the reranker isn't loaded (offline first run / cooldown) or fails. Display
/// scores are left as the bi-encoder's normalized values (comparable across
/// result sets); only the order changes.
fn rerank_chapters(
    state: &AppState,
    query: &str,
    ranked: Vec<RankedChapterHit>,
) -> Vec<RankedChapterHit> {
    let Some(rerank) = state.reranker.get() else {
        return ranked;
    };
    let texts: Vec<&str> = ranked.iter().map(|h| h.7.as_str()).collect();
    match rerank(query, &texts) {
        Ok(scores) => {
            if scores.len() != ranked.len() {
                eprintln!(
                    "RERANK: score count mismatch ({} vs {}); keeping bi-encoder order",
                    scores.len(),
                    ranked.len()
                );
                return ranked;
            }
            let mut scored: Vec<(f32, RankedChapterHit)> = ranked
                .into_iter()
                .zip(scores)
                .map(|(h, s)| (s, h))
                .collect();
            scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
            scored.into_iter().map(|(_, h)| h).collect()
        }
        Err(e) => {
            eprintln!("RERANK: cross-encoder failed: {e}");
            ranked
        }
    }
}

/// Hybrid chapter ranking: fuses the semantic (bi-encoder) top-50 with the
/// BM25 top-50 via Reciprocal Rank Fusion, then cross-encoder reranks the fused
/// top-50. Keyword queries (exact names, rare terms) that cosine misses now
/// surface through the BM25 lane; the rerank then refines the fused order.
///
/// The fused hits are mapped back through the *unfiltered* best-chunk list so
/// BM25-only chapters below the semantic score floor still participate; their
/// raw cosine scores are normalized to the display scale before reranking.
///
/// The per-book diversity cap is applied *post-fusion* (before the rerank): the
/// semantic lane is already capped, but the BM25 lane is not, so a dominant
/// book with many keyword-matching chapters could otherwise flood the fused
/// top-50. Capping before the rerank also means the expensive cross-encoder
/// scores a diverse candidate set. The cap is a no-op for the single-book
/// drill-down window (all hits backfill), so `book_url` searches stay uncapped.
fn fuse_and_rerank(
    state: &AppState,
    query: &str,
    chapters: &[crate::embeddings::ChunkHit],
    query_vec: &[f32],
) -> Vec<RankedChapterHit> {
    // Semantic lane: top-50 (floored, per-book capped, normalized).
    let semantic = crate::embeddings::rank_chapters(chapters, query_vec, RERANK_CANDIDATES);
    let semantic_urls: Vec<String> = semantic.iter().map(|h| h.2.clone()).collect();

    // Keyword lane: BM25 top-50 over the same corpus. The returned best-chunk
    // text is the reranker passage for below-floor chapters.
    let bm25 = crate::bm25::Bm25Index::build(chapters);
    let bm25_hits = bm25.search(query, RERANK_CANDIDATES);
    let bm25_urls: Vec<String> = bm25_hits.iter().map(|(_, url, _)| url.clone()).collect();
    let bm25_best_text: HashMap<String, String> = bm25_hits
        .into_iter()
        .map(|(_, url, text)| (url, text))
        .collect();

    // Fuse, then map URLs back to hits via the unfiltered best-chunk list.
    let fused = crate::bm25::rrf_fuse(&[semantic_urls, bm25_urls], RERANK_CANDIDATES);
    let all_best = crate::embeddings::best_chunks_per_chapter(chapters, query_vec);
    let by_url: HashMap<String, RankedChapterHit> =
        all_best.into_iter().map(|h| (h.2.clone(), h)).collect();
    let mut fused_hits: Vec<RankedChapterHit> = fused
        .into_iter()
        .filter_map(|url| {
            let mut hit = by_url.get(&url)?.clone();
            // For a below-floor chapter surfaced by BM25, the highest-cosine
            // chunk may be unrelated to the matched keyword — use the
            // BM25-best chunk as the reranker passage instead.
            if hit.6 < crate::embeddings::SCORE_FLOOR
                && let Some(text) = bm25_best_text.get(&url)
            {
                hit.7 = text.clone();
            }
            Some(hit)
        })
        .collect();
    for hit in &mut fused_hits {
        hit.6 = crate::embeddings::normalize_score(hit.6);
    }

    // Per-book cap post-fusion (see the doc comment above), then restore
    // descending score order before the rerank.
    fused_hits = crate::embeddings::cap_per_book(fused_hits, crate::embeddings::MAX_PER_BOOK);
    fused_hits.truncate(RERANK_CANDIDATES);
    fused_hits.sort_by(|a, b| b.6.partial_cmp(&a.6).unwrap_or(std::cmp::Ordering::Equal));

    rerank_chapters(state, query, fused_hits)
}

/// Cross-encoder rerank for books: each book's candidate text is its best
/// chapter's best chunk. Books with no chapter hits have no reranker passage —
/// they are excluded from reranking and kept at the end in their original
/// order, rather than scoring empty text and getting arbitrary logits. Falls
/// back to the bi-encoder order when the reranker isn't loaded or fails.
fn rerank_books(
    state: &AppState,
    query: &str,
    ranked_books: Vec<RankedBook>,
    by_book: &HashMap<String, Vec<RankedChapterHit>>,
) -> Vec<RankedBook> {
    let Some(rerank) = state.reranker.get() else {
        return ranked_books;
    };
    // Partition into books with a reranker passage (a best chapter hit) and
    // those without — the latter keep their original relative order at the end.
    let mut with_passage: Vec<(RankedBook, &str)> = Vec::new();
    let mut no_passage: Vec<RankedBook> = Vec::new();
    for book in ranked_books {
        match by_book.get(&book.0).and_then(|g| g.first()) {
            Some(h) => with_passage.push((book, h.7.as_str())),
            None => no_passage.push(book),
        }
    }
    if with_passage.is_empty() {
        return no_passage;
    }
    let texts: Vec<&str> = with_passage.iter().map(|(_, t)| *t).collect();
    let scores = match rerank(query, &texts) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("RERANK: cross-encoder failed: {e}");
            let mut out: Vec<RankedBook> = with_passage.into_iter().map(|(b, _)| b).collect();
            out.extend(no_passage);
            return out;
        }
    };
    if scores.len() != texts.len() {
        eprintln!(
            "RERANK: score count mismatch ({} vs {}); keeping bi-encoder order",
            scores.len(),
            texts.len()
        );
        let mut out: Vec<RankedBook> = with_passage.into_iter().map(|(b, _)| b).collect();
        out.extend(no_passage);
        return out;
    }
    let mut scored: Vec<(f32, RankedBook)> = with_passage
        .into_iter()
        .zip(scores)
        .map(|((b, _), s)| (s, b))
        .collect();
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut out: Vec<RankedBook> = scored.into_iter().map(|(_, b)| b).collect();
    out.extend(no_passage);
    out
}
