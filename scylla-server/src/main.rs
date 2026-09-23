mod bm25;
mod db;
mod embeddings;
mod routes;
mod snippet;
mod state;

use std::sync::Arc;

use axum::Router;
use state::AppState;
use tokio::sync::Mutex;

/// Work item for the dedicated embedding thread.
#[derive(Clone)]
enum EmbedRequest {
    Chapter {
        chapter_url: String,
        text: String,
        /// The originating EmbedBatch job, if any (used to report back the
        /// "Embedded" status).
        job_id: Option<scylla_core::types::JobId>,
    },
    Description {
        book_url: String,
        text: String,
    },
}

/// Max requests collected into one batched forward pass.
const EMBED_BATCH_SIZE: usize = 8;
/// How long to wait for more requests before flushing a partial batch.
const EMBED_BATCH_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(50);

#[tokio::main]
async fn main() {
    let port: u16 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(8080);

    let db_path = dirs::data_local_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("scylla-reader")
        .join("library.db");

    let db = Arc::new(Mutex::new(
        db::ServerDb::open_path(&db_path).expect("Failed to open database"),
    ));

    let (cmd_tx, cmd_rx) = std::sync::mpsc::channel();
    let (event_tx, event_rx) = std::sync::mpsc::channel();
    let embed_event_tx = event_tx.clone();

    let jobs: Arc<std::sync::Mutex<Vec<scylla_core::types::JobDto>>> =
        Arc::new(std::sync::Mutex::new(Vec::new()));
    let (job_events_tx, _) =
        tokio::sync::broadcast::channel::<Arc<scylla_core::messenger::AppEvent>>(256);

    let registry = Arc::new(std::sync::Mutex::new(
        scylla_core::scraper::ScraperRegistry::new(),
    ));

    let max_workers: u8 = 4;
    let rate_limit: u64 = 2;
    let worker_registry = registry.clone();
    std::thread::spawn(move || {
        let manager = scylla_core::worker::JobManager::new(
            cmd_rx,
            event_tx,
            worker_registry,
            max_workers,
            rate_limit,
        );
        manager.run();
    });

    let event_db = db.clone();
    let event_jobs = jobs.clone();
    let event_broadcast = job_events_tx.clone();
    let event_registry = registry.clone();
    let autoembed = Arc::new(std::sync::Mutex::new(false));
    let event_autoembed = autoembed.clone();
    let (embed_tx, embed_rx) = std::sync::mpsc::channel::<EmbedRequest>();
    let embed_db = db.clone();
    let shared_embedder = Arc::new(embeddings::SharedEmbedder::new());
    let thread_embedder = shared_embedder.clone();
    let thread_event_tx = embed_event_tx.clone();
    std::thread::spawn(move || {
        let mut chapter_counts: std::collections::HashMap<String, u32> =
            std::collections::HashMap::new();
        let mut genre_embeddings: Option<Vec<(String, Vec<f32>)>> = None;

        loop {
            // Collect a batch of requests (block on the first, then drain more
            // with a short timeout up to EMBED_BATCH_SIZE).
            let mut batch = Vec::new();
            match embed_rx.recv() {
                Ok(req) => batch.push(req),
                Err(_) => break, // channel closed
            }
            while batch.len() < EMBED_BATCH_SIZE {
                match embed_rx.recv_timeout(EMBED_BATCH_TIMEOUT) {
                    Ok(req) => batch.push(req),
                    Err(_) => break,
                }
            }
            process_embed_batch(
                &batch,
                &embed_db,
                &thread_embedder,
                &thread_event_tx,
                &mut chapter_counts,
                &mut genre_embeddings,
            );
        }
    });

    // Startup backfill: enqueue descriptions for books that have one but no
    // description embedding yet.
    {
        let db = db.lock().await;
        match db.books_missing_description_embedding() {
            Ok(books) => {
                eprintln!(
                    "Backfilling {} book description(s) for embedding",
                    books.len()
                );
                for (book_url, desc) in books {
                    let _ = embed_tx.send(EmbedRequest::Description {
                        book_url,
                        text: desc,
                    });
                }
            }
            Err(e) => eprintln!("Failed to load books for embedding backfill: {e}"),
        }
    }

    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create event runtime");
        for event in event_rx {
            let emit = routes::jobs::update_job_snapshot(&event_jobs, &event);
            routes::jobs::reload_registry_on_plugin_installed(&event_registry, &event);
            if let scylla_core::messenger::AppEvent::ChapterFetched(chapter) = &event {
                // Automatic embedding is gated on the autoembed setting.
                if *event_autoembed.lock().unwrap() {
                    let _ = embed_tx.send(EmbedRequest::Chapter {
                        chapter_url: chapter.url.clone(),
                        text: chapter.content.clone(),
                        job_id: None,
                    });
                }
            }
            if let scylla_core::messenger::AppEvent::ChapterToEmbed(job_id, chapter) = &event {
                // EmbedBatch jobs must embed regardless of the autoembed setting.
                let _ = embed_tx.send(EmbedRequest::Chapter {
                    chapter_url: chapter.url.clone(),
                    text: chapter.content.clone(),
                    job_id: Some(*job_id),
                });
            }
            if let scylla_core::messenger::AppEvent::BookScraped(book) = &event {
                let db = event_db.clone();
                let book_for_db = book.clone();
                let result = rt.block_on(async move {
                    let db = db.lock().await;
                    db.upsert_book(&book_for_db)
                });
                if let Err(e) = result {
                    eprintln!("Failed to persist scraped book: {e}");
                }
                if *event_autoembed.lock().unwrap()
                    && let Some(desc) = &book.description
                {
                    let _ = embed_tx.send(EmbedRequest::Description {
                        book_url: book.url.clone(),
                        text: desc.clone(),
                    });
                }
            }
            let _ = event_broadcast.send(Arc::new(event));
            if let Some(emit) = emit {
                let _ = event_broadcast.send(Arc::new(emit));
            }
        }
    });

    let app_state = Arc::new(AppState {
        db,
        cmd_tx,
        registry,
        max_workers: std::sync::Mutex::new(max_workers),
        rate_limit: std::sync::Mutex::new(rate_limit),
        jobs,
        job_events: job_events_tx,
        embedder: shared_embedder,
        reranker: Arc::new(embeddings::SharedCrossEncoder::new()),
        autoembed,
    });

    let app = build_router(app_state);

    let addr = format!("127.0.0.1:{}", port);
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

fn build_router(app_state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/health", axum::routing::get(routes::health::health))
        .route(
            "/api/books",
            axum::routing::get(routes::books::list_books).post(routes::books::create_book),
        )
        .route(
            "/api/books/:url",
            axum::routing::get(routes::books::get_book)
                .put(routes::books::upsert_book)
                .delete(routes::books::delete_book),
        )
        .route(
            "/api/books/:url/status",
            axum::routing::patch(routes::books::update_status),
        )
        .route(
            "/api/books/:url/crawl",
            axum::routing::post(routes::books::crawl_book),
        )
        .route(
            "/api/books/:url/embedding-status",
            axum::routing::get(routes::books::embedding_status),
        )
        .route(
            "/api/books/:url/sessions",
            axum::routing::get(routes::sessions::list_sessions)
                .post(routes::sessions::create_session),
        )
        .route(
            "/api/books/:url/active-session",
            axum::routing::patch(routes::sessions::set_active_session),
        )
        .route(
            "/api/sessions/:id/progress",
            axum::routing::patch(routes::sessions::update_progress),
        )
        .route(
            "/api/sessions/:id",
            axum::routing::patch(routes::sessions::rename_session)
                .delete(routes::sessions::delete_session),
        )
        .route(
            "/api/books/:book_url/chapters/:chapter_url/content",
            axum::routing::get(routes::chapters::get_chapter_content),
        )
        .route(
            "/api/settings",
            axum::routing::get(routes::settings::get_settings)
                .patch(routes::settings::update_settings),
        )
        .route("/api/jobs", axum::routing::get(routes::jobs::list_jobs))
        .route(
            "/api/jobs/stream",
            axum::routing::get(routes::jobs::stream_jobs),
        )
        .route(
            "/api/jobs/cancel",
            axum::routing::post(routes::jobs::cancel_job),
        )
        .route(
            "/api/jobs/cancel-all",
            axum::routing::post(routes::jobs::cancel_all),
        )
        .route(
            "/api/jobs/retry",
            axum::routing::post(routes::jobs::retry_job),
        )
        .route(
            "/api/jobs/retry-all",
            axum::routing::post(routes::jobs::retry_all),
        )
        .route(
            "/api/jobs/flush-completed",
            axum::routing::post(routes::jobs::flush_completed),
        )
        .route(
            "/api/jobs/flush-all",
            axum::routing::post(routes::jobs::flush_all),
        )
        .route(
            "/api/jobs/workers",
            axum::routing::post(routes::jobs::set_workers),
        )
        .route(
            "/api/jobs/rate-limit",
            axum::routing::post(routes::jobs::set_rate_limit),
        )
        .route(
            "/api/jobs/enqueue",
            axum::routing::post(routes::jobs::enqueue_job),
        )
        .route(
            "/api/plugins/install",
            axum::routing::post(routes::plugins::install_plugin),
        )
        .route("/api/search", axum::routing::post(routes::search::search))
        .layer(tower_http::cors::CorsLayer::permissive())
        .with_state(app_state)
}

/// Recomputes a book's aggregate embedding + genre classification.
///
/// Genre embeddings (first classification embeds ~20 genres) are computed
/// OUTSIDE the DB lock so HTTP handlers never stall on them.
fn recompute_aggregate(
    db: &Arc<tokio::sync::Mutex<db::ServerDb>>,
    book_url: &str,
    embed: &embeddings::EmbedFn,
    genre_embeddings: &mut Option<Vec<(String, Vec<f32>)>>,
) {
    let (desc, chapters) = {
        let db = db.blocking_lock();
        let desc = match db.get_book_embedding(book_url) {
            Ok(Some((desc, _, _))) => desc,
            _ => None,
        };
        let chapters = match db.load_chapter_embeddings_for_book(book_url) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Failed to load chapter embeddings: {e}");
                return;
            }
        };
        (desc, chapters)
    };
    let agg = embeddings::aggregate(desc.as_deref(), &chapters);
    if agg.is_empty() {
        return;
    }
    let genres = match genre_embeddings {
        Some(g) => g.clone(),
        None => {
            let mut g = Vec::new();
            for (name, desc_text) in embeddings::GENRES {
                match embed(&[desc_text], embeddings::EmbedMode::Passage) {
                    Ok(v) => g.push((name.to_string(), v[0].clone())),
                    Err(e) => eprintln!("Failed to embed genre {name}: {e}"),
                }
            }
            *genre_embeddings = Some(g.clone());
            g
        }
    };
    let top = embeddings::classify(&agg, &genres, 3);
    let db = db.blocking_lock();
    if let Err(e) = db.upsert_book_embedding(book_url, desc.as_deref(), Some(&agg), Some(&top)) {
        eprintln!("Failed to store aggregate embedding: {e}");
    }
}

/// Whether a book's aggregate should be recomputed after embedding `count`
/// chapters: on the first chapter (so <5-chapter books still get one) and then
/// every 5th.
fn should_recompute(count: u32) -> bool {
    count == 1 || count.is_multiple_of(5)
}

/// Stable content hash used to skip re-embedding unchanged chapters.
///
/// FNV-1a (stable across Rust releases, unlike `DefaultHasher`) over the text
/// with the embedding model version folded in: a model swap changes every hash,
/// so all chapters are re-embedded exactly once instead of silently mixing
/// vectors from different models.
fn content_hash(text: &str) -> String {
    const FNV_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;
    let mut hash = FNV_OFFSET_BASIS;
    for byte in embeddings::MODEL_VERSION
        .as_bytes()
        .iter()
        .chain(text.as_bytes())
    {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    format!("{:016x}", hash)
}

/// Whether a chapter should be skipped because it's already embedded with the
/// same content hash and the same number of chunks.
fn should_skip_embedding(
    db: &Arc<tokio::sync::Mutex<db::ServerDb>>,
    chapter_url: &str,
    hash: &str,
    expected_chunks: usize,
) -> bool {
    db.blocking_lock()
        .chapter_chunk_state(chapter_url)
        .ok()
        .map(|(count, existing)| count == expected_chunks && existing.as_deref() == Some(hash))
        .unwrap_or(false)
}

/// Embeds `texts` in sub-batches of `size` (bounds model memory when a batch
/// of chapters expands into many chunks).
fn embed_in_batches(
    embed: &embeddings::EmbedFn,
    texts: &[&str],
    size: usize,
) -> anyhow::Result<Vec<Vec<f32>>> {
    let mut out = Vec::with_capacity(texts.len());
    for chunk in texts.chunks(size) {
        out.extend(embed(chunk, embeddings::EmbedMode::Passage)?);
    }
    Ok(out)
}

/// A chapter pending embedding: its chunks, content hash, and originating job.
struct PendingChapter {
    chapter_url: String,
    job_id: Option<scylla_core::types::JobId>,
    hash: String,
    chunks: Vec<String>,
}

/// A book description pending embedding.
struct PendingDescription {
    book_url: String,
    text: String,
}

/// Embeds a collected batch of requests in forward passes, then stores each
/// result. Chapters are chunked (512-token windows), each chunk embedded, and
/// the chapter's chunks stored atomically.
fn process_embed_batch(
    batch: &[EmbedRequest],
    embed_db: &Arc<tokio::sync::Mutex<db::ServerDb>>,
    thread_embedder: &embeddings::SharedEmbedder,
    event_tx: &std::sync::mpsc::Sender<scylla_core::messenger::AppEvent>,
    chapter_counts: &mut std::collections::HashMap<String, u32>,
    genre_embeddings: &mut Option<Vec<(String, Vec<f32>)>>,
) {
    let batch_start = std::time::Instant::now();
    let mut db_time = std::time::Duration::ZERO;

    let Some(embed) = thread_embedder.get() else {
        eprintln!(
            "EMBED: embedder unavailable (model load failed or in cooldown) — \
             skipping batch of {}",
            batch.len()
        );
        return;
    };
    let Some(chunker) = thread_embedder.get_chunker() else {
        eprintln!("EMBED: chunker unavailable — skipping batch");
        return;
    };

    // Chunk + skip-check chapters; collect descriptions.
    let mut pending_chapters: Vec<PendingChapter> = Vec::new();
    let mut pending_descriptions: Vec<PendingDescription> = Vec::new();
    for req in batch {
        match req {
            EmbedRequest::Chapter {
                chapter_url,
                text,
                job_id,
            } => {
                let chunks = chunker(text);
                let hash = content_hash(text);
                let db_start = std::time::Instant::now();
                let skip = should_skip_embedding(embed_db, chapter_url, &hash, chunks.len());
                db_time += db_start.elapsed();
                if skip {
                    eprintln!("EMBED: chapter {chapter_url} already embedded, skipping");
                    continue;
                }
                pending_chapters.push(PendingChapter {
                    chapter_url: chapter_url.clone(),
                    job_id: *job_id,
                    hash,
                    chunks,
                });
            }
            EmbedRequest::Description { book_url, text } => {
                pending_descriptions.push(PendingDescription {
                    book_url: book_url.clone(),
                    text: text.clone(),
                });
            }
        }
    }
    if pending_chapters.is_empty() && pending_descriptions.is_empty() {
        return;
    }

    // Flatten all texts (chunks + descriptions) and embed in sub-batches.
    let mut texts: Vec<&str> = Vec::new();
    for p in &pending_chapters {
        for c in &p.chunks {
            texts.push(c.as_str());
        }
    }
    for d in &pending_descriptions {
        texts.push(d.text.as_str());
    }
    let model_start = std::time::Instant::now();
    let embeddings = match embed_in_batches(&embed, &texts, EMBED_BATCH_SIZE) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("EMBED: batch embedding failed ({} texts): {e}", texts.len());
            // Report the failure back for every pending chapter with a job id.
            for p in &pending_chapters {
                if let Some(job_id) = p.job_id {
                    let _ = event_tx.send(scylla_core::messenger::AppEvent::ChapterEmbeddedFailed(
                        job_id,
                        p.chapter_url.clone(),
                    ));
                }
            }
            return;
        }
    };
    let model_time = model_start.elapsed();

    // Store chapters atomically (all chunks or none).
    let mut offset = 0;
    for p in &pending_chapters {
        eprintln!("EMBED: embedding chapter {}", p.chapter_url);
        let chunk_embeddings = &embeddings[offset..offset + p.chunks.len()];
        offset += p.chunks.len();
        let db_start = std::time::Instant::now();
        let book_url = embed_db
            .blocking_lock()
            .find_book_url_for_chapter(&p.chapter_url)
            .ok()
            .flatten();
        if book_url.is_none() {
            eprintln!(
                "EMBED: no book found for chapter {} — \
                 storing without book association (embedding-status will not count it)",
                p.chapter_url
            );
        }
        let chunk_rows: Vec<(usize, &str, &[f32])> = p
            .chunks
            .iter()
            .enumerate()
            .map(|(i, c)| (i, c.as_str(), chunk_embeddings[i].as_slice()))
            .collect();
        let db = embed_db.blocking_lock();
        if let Err(e) = db.upsert_chapter_chunks(
            &p.chapter_url,
            book_url.as_deref(),
            &chunk_rows,
            Some(&p.hash),
        ) {
            eprintln!("EMBED: failed to store chapter chunks: {e}");
            if let Some(job_id) = p.job_id {
                let _ = event_tx.send(scylla_core::messenger::AppEvent::ChapterEmbeddedFailed(
                    job_id,
                    p.chapter_url.clone(),
                ));
            }
            continue;
        }
        drop(db);
        db_time += db_start.elapsed();
        eprintln!(
            "EMBED: stored {} chunks for chapter {} (book: {:?})",
            p.chunks.len(),
            p.chapter_url,
            book_url
        );
        if let Some(job_id) = p.job_id {
            let _ = event_tx.send(scylla_core::messenger::AppEvent::ChapterEmbedded(
                job_id,
                p.chapter_url.clone(),
            ));
        }
        if let Some(book_url) = book_url {
            let count = chapter_counts.entry(book_url.clone()).or_insert(0);
            *count += 1;
            if should_recompute(*count) {
                recompute_aggregate(embed_db, &book_url, &embed, genre_embeddings);
            }
        }
    }

    // Store descriptions.
    for d in &pending_descriptions {
        eprintln!("EMBED: embedding description for {}", d.book_url);
        let embedding = &embeddings[offset];
        offset += 1;
        let db_start = std::time::Instant::now();
        let db = embed_db.blocking_lock();
        if let Err(e) = db.upsert_book_embedding(&d.book_url, Some(embedding), None, None) {
            eprintln!("EMBED: failed to store description embedding: {e}");
            continue;
        }
        drop(db);
        db_time += db_start.elapsed();
        eprintln!("EMBED: stored description embedding for {}", d.book_url);
        recompute_aggregate(embed_db, &d.book_url, &embed, genre_embeddings);
    }

    eprintln!(
        "EMBED: batch {} chapters: model {}ms, db {}ms, total {}ms",
        pending_chapters.len(),
        model_time.as_millis(),
        db_time.as_millis(),
        batch_start.elapsed().as_millis()
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    const BOOK_PATH: &str = "/api/books/http%3A%2F%2Fexample.com%2Fbook";

    fn test_app() -> Router {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let db = Arc::new(Mutex::new(db::ServerDb::open_conn(conn).unwrap()));
        let (cmd_tx, _cmd_rx) = std::sync::mpsc::channel();
        let registry = Arc::new(std::sync::Mutex::new(
            scylla_core::scraper::ScraperRegistry::new(),
        ));
        let state =
            Arc::new(AppState {
                db,
                cmd_tx,
                registry,
                max_workers: std::sync::Mutex::new(4),
                rate_limit: std::sync::Mutex::new(2),
                jobs: Arc::new(std::sync::Mutex::new(Vec::new())),
                job_events:
                    tokio::sync::broadcast::channel::<Arc<scylla_core::messenger::AppEvent>>(256).0,
                embedder: Arc::new(embeddings::SharedEmbedder::new()),
                reranker: Arc::new(embeddings::SharedCrossEncoder::in_cooldown()),
                autoembed: Arc::new(std::sync::Mutex::new(false)),
            });
        build_router(state)
    }

    fn test_app_with_cmd_rx() -> (
        Router,
        std::sync::mpsc::Receiver<scylla_core::messenger::AppCommand>,
    ) {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let db = Arc::new(Mutex::new(db::ServerDb::open_conn(conn).unwrap()));
        let (cmd_tx, cmd_rx) = std::sync::mpsc::channel();
        let registry = Arc::new(std::sync::Mutex::new(
            scylla_core::scraper::ScraperRegistry::new(),
        ));
        let state =
            Arc::new(AppState {
                db,
                cmd_tx,
                registry,
                max_workers: std::sync::Mutex::new(4),
                rate_limit: std::sync::Mutex::new(2),
                jobs: Arc::new(std::sync::Mutex::new(Vec::new())),
                job_events:
                    tokio::sync::broadcast::channel::<Arc<scylla_core::messenger::AppEvent>>(256).0,
                embedder: Arc::new(embeddings::SharedEmbedder::new()),
                reranker: Arc::new(embeddings::SharedCrossEncoder::in_cooldown()),
                autoembed: Arc::new(std::sync::Mutex::new(false)),
            });
        (build_router(state), cmd_rx)
    }

    fn test_app_with_embed_and_db(
        embed: embeddings::EmbedFn,
    ) -> (Router, Arc<Mutex<db::ServerDb>>) {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let db = Arc::new(Mutex::new(db::ServerDb::open_conn(conn).unwrap()));
        let (cmd_tx, _cmd_rx) = std::sync::mpsc::channel();
        let registry = Arc::new(std::sync::Mutex::new(
            scylla_core::scraper::ScraperRegistry::new(),
        ));
        let state =
            Arc::new(AppState {
                db: db.clone(),
                cmd_tx,
                registry,
                max_workers: std::sync::Mutex::new(4),
                rate_limit: std::sync::Mutex::new(2),
                jobs: Arc::new(std::sync::Mutex::new(Vec::new())),
                job_events:
                    tokio::sync::broadcast::channel::<Arc<scylla_core::messenger::AppEvent>>(256).0,
                embedder: Arc::new(embeddings::SharedEmbedder::with_embed(embed)),
                reranker: Arc::new(embeddings::SharedCrossEncoder::in_cooldown()),
                autoembed: Arc::new(std::sync::Mutex::new(false)),
            });
        (build_router(state), db)
    }

    fn test_app_with_embed_rerank_and_db(
        embed: embeddings::EmbedFn,
        rerank: embeddings::RerankFn,
    ) -> (Router, Arc<Mutex<db::ServerDb>>) {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let db = Arc::new(Mutex::new(db::ServerDb::open_conn(conn).unwrap()));
        let (cmd_tx, _cmd_rx) = std::sync::mpsc::channel();
        let registry = Arc::new(std::sync::Mutex::new(
            scylla_core::scraper::ScraperRegistry::new(),
        ));
        let state =
            Arc::new(AppState {
                db: db.clone(),
                cmd_tx,
                registry,
                max_workers: std::sync::Mutex::new(4),
                rate_limit: std::sync::Mutex::new(2),
                jobs: Arc::new(std::sync::Mutex::new(Vec::new())),
                job_events:
                    tokio::sync::broadcast::channel::<Arc<scylla_core::messenger::AppEvent>>(256).0,
                embedder: Arc::new(embeddings::SharedEmbedder::with_embed(embed)),
                reranker: Arc::new(embeddings::SharedCrossEncoder::with_rerank(rerank)),
                autoembed: Arc::new(std::sync::Mutex::new(false)),
            });
        (build_router(state), db)
    }

    fn test_app_with_embedder_in_cooldown() -> Router {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let db = Arc::new(Mutex::new(db::ServerDb::open_conn(conn).unwrap()));
        let (cmd_tx, _cmd_rx) = std::sync::mpsc::channel();
        let registry = Arc::new(std::sync::Mutex::new(
            scylla_core::scraper::ScraperRegistry::new(),
        ));
        let state =
            Arc::new(AppState {
                db,
                cmd_tx,
                registry,
                max_workers: std::sync::Mutex::new(4),
                rate_limit: std::sync::Mutex::new(2),
                jobs: Arc::new(std::sync::Mutex::new(Vec::new())),
                job_events:
                    tokio::sync::broadcast::channel::<Arc<scylla_core::messenger::AppEvent>>(256).0,
                embedder: Arc::new(embeddings::SharedEmbedder::in_cooldown()),
                reranker: Arc::new(embeddings::SharedCrossEncoder::in_cooldown()),
                autoembed: Arc::new(std::sync::Mutex::new(false)),
            });
        build_router(state)
    }

    fn test_app_with_embedder_in_cooldown_and_db() -> (Router, Arc<Mutex<db::ServerDb>>) {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let db = Arc::new(Mutex::new(db::ServerDb::open_conn(conn).unwrap()));
        let (cmd_tx, _cmd_rx) = std::sync::mpsc::channel();
        let registry = Arc::new(std::sync::Mutex::new(
            scylla_core::scraper::ScraperRegistry::new(),
        ));
        let state =
            Arc::new(AppState {
                db: db.clone(),
                cmd_tx,
                registry,
                max_workers: std::sync::Mutex::new(4),
                rate_limit: std::sync::Mutex::new(2),
                jobs: Arc::new(std::sync::Mutex::new(Vec::new())),
                job_events:
                    tokio::sync::broadcast::channel::<Arc<scylla_core::messenger::AppEvent>>(256).0,
                embedder: Arc::new(embeddings::SharedEmbedder::in_cooldown()),
                reranker: Arc::new(embeddings::SharedCrossEncoder::in_cooldown()),
                autoembed: Arc::new(std::sync::Mutex::new(false)),
            });
        (build_router(state), db)
    }

    fn sample_book_json() -> serde_json::Value {
        serde_json::json!({
            "title": "Test Book",
            "url": "http://example.com/book",
            "status": "Reading",
            "sessions": [],
            "active_session_id": null,
            "tags": ["tag1"],
            "cover_url": null,
            "description": "desc",
            "chapters": []
        })
    }

    async fn body_json(resp: axum::response::Response) -> serde_json::Value {
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice(&body).unwrap()
    }

    #[tokio::test]
    async fn test_put_book_upserts_and_get_returns_it() {
        let app = test_app();

        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(BOOK_PATH)
                    .header("content-type", "application/json")
                    .body(Body::from(sample_book_json().to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let resp = app
            .oneshot(
                Request::builder()
                    .uri(BOOK_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp).await;
        assert_eq!(json["url"], "http://example.com/book");
        assert_eq!(json["title"], "Test Book");
        assert_eq!(json["tags"][0], "tag1");
    }

    #[tokio::test]
    async fn test_put_book_bad_body_returns_400() {
        let app = test_app();
        let resp = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(BOOK_PATH)
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"not":"a book"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_put_book_path_mismatch_returns_400() {
        let app = test_app();
        let mut book = sample_book_json();
        book["url"] = serde_json::json!("http://other.example.com/book");
        let resp = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(BOOK_PATH)
                    .header("content-type", "application/json")
                    .body(Body::from(book.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_rename_and_delete_missing_session_returns_404() {
        let app = test_app();
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri("/api/sessions/999999")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"name":"x"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        let resp = app
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/api/sessions/999999")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_create_session_and_list() {
        let app = test_app();

        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(BOOK_PATH)
                    .header("content-type", "application/json")
                    .body(Body::from(sample_book_json().to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("{}/sessions", BOOK_PATH))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"name":"s1","total":10}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        let session = body_json(resp).await;
        assert_eq!(session["name"], "s1");
        assert_eq!(session["progress"]["total"], 10);
        let id = session["id"].as_i64().unwrap();

        let resp = app
            .oneshot(
                Request::builder()
                    .uri(format!("{}/sessions", BOOK_PATH))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let sessions = body_json(resp).await;
        let list = sessions.as_array().unwrap();
        assert!(list.iter().any(|s| s["id"] == id && s["name"] == "s1"));
    }

    #[tokio::test]
    async fn test_rename_and_delete_session() {
        let app = test_app();

        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(BOOK_PATH)
                    .header("content-type", "application/json")
                    .body(Body::from(sample_book_json().to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("{}/sessions", BOOK_PATH))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"name":"s1","total":10}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        let session = body_json(resp).await;
        let id = session["id"].as_i64().unwrap();

        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri(format!("/api/sessions/{}", id))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"name":"renamed"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("{}/sessions", BOOK_PATH))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let sessions = body_json(resp).await;
        let renamed = sessions
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["id"] == id)
            .unwrap();
        assert_eq!(renamed["name"], "renamed");

        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/api/sessions/{}", id))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);

        let resp = app
            .oneshot(
                Request::builder()
                    .uri(format!("{}/sessions", BOOK_PATH))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let sessions = body_json(resp).await;
        assert!(!sessions.as_array().unwrap().iter().any(|s| s["id"] == id));
    }

    #[tokio::test]
    async fn test_update_status() {
        let app = test_app();

        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(BOOK_PATH)
                    .header("content-type", "application/json")
                    .body(Body::from(sample_book_json().to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri(format!("{}/status", BOOK_PATH))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"status":"Dropped"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let resp = app
            .oneshot(
                Request::builder()
                    .uri(BOOK_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let json = body_json(resp).await;
        assert_eq!(json["status"], "Dropped");
    }

    #[tokio::test]
    async fn test_get_settings_returns_state_values() {
        let app = test_app();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/settings")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp).await;
        assert_eq!(json["max_workers"], 4);
        assert_eq!(json["rate_limit"], 2);
        assert_eq!(json["autoembed"], false);
    }

    #[tokio::test]
    async fn test_update_settings_updates_state() {
        let app = test_app();
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri("/api/settings")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"max_workers":8,"rate_limit":5,"autoembed":true}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/settings")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let json = body_json(resp).await;
        assert_eq!(json["max_workers"], 8);
        assert_eq!(json["rate_limit"], 5);
        assert_eq!(json["autoembed"], true);
    }

    #[tokio::test]
    async fn test_update_settings_rejects_invalid_max_workers() {
        let app = test_app();
        for body in [r#"{"max_workers":0}"#, r#"{"max_workers":256}"#] {
            let resp = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("PATCH")
                        .uri("/api/settings")
                        .header("content-type", "application/json")
                        .body(Body::from(body))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        }

        // State unchanged after rejected updates.
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/settings")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let json = body_json(resp).await;
        assert_eq!(json["max_workers"], 4);
    }

    #[tokio::test]
    async fn test_get_jobs_returns_snapshot() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let db = Arc::new(Mutex::new(db::ServerDb::open_conn(conn).unwrap()));
        let (cmd_tx, _cmd_rx) = std::sync::mpsc::channel();
        let registry = Arc::new(std::sync::Mutex::new(
            scylla_core::scraper::ScraperRegistry::new(),
        ));
        let jobs = Arc::new(std::sync::Mutex::new(vec![scylla_core::types::JobDto {
            id: 1,
            kind: "Scrape".into(),
            status: "Queued".into(),
            target: "http://example.com".into(),
            priority: "Normal".into(),
            chapter_idx: None,
            created_at_ms: 0,
            started_at_ms: None,
            completed_at_ms: None,
            error: None,
            outcome: None,
            detail: None,
        }]));
        let state =
            Arc::new(AppState {
                db,
                cmd_tx,
                registry,
                max_workers: std::sync::Mutex::new(4),
                rate_limit: std::sync::Mutex::new(2),
                jobs,
                job_events:
                    tokio::sync::broadcast::channel::<Arc<scylla_core::messenger::AppEvent>>(256).0,
                embedder: Arc::new(embeddings::SharedEmbedder::new()),
                reranker: Arc::new(embeddings::SharedCrossEncoder::in_cooldown()),
                autoembed: Arc::new(std::sync::Mutex::new(false)),
            });
        let app = build_router(state);

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/jobs")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp).await;
        assert!(json["server_now_ms"].is_u64());
        let list = json["jobs"].as_array().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0]["id"], 1);
        assert_eq!(list[0]["kind"], "Scrape");
        assert_eq!(list[0]["status"], "Queued");
    }

    #[tokio::test]
    async fn test_cancel_job_sends_command() {
        let (app, cmd_rx) = test_app_with_cmd_rx();
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/jobs/cancel")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"id":7}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let cmd = cmd_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap();
        assert!(matches!(
            cmd,
            scylla_core::messenger::AppCommand::CancelJob(7)
        ));
    }

    #[tokio::test]
    async fn test_cancel_all_sends_command() {
        let (app, cmd_rx) = test_app_with_cmd_rx();
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/jobs/cancel-all")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let cmd = cmd_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap();
        assert!(matches!(cmd, scylla_core::messenger::AppCommand::CancelAll));
    }

    #[tokio::test]
    async fn test_retry_job_sends_command() {
        let (app, cmd_rx) = test_app_with_cmd_rx();
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/jobs/retry")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"id":9}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let cmd = cmd_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap();
        assert!(matches!(
            cmd,
            scylla_core::messenger::AppCommand::RetryJob(9)
        ));
    }

    #[tokio::test]
    async fn test_retry_all_sends_command() {
        let (app, cmd_rx) = test_app_with_cmd_rx();
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/jobs/retry-all")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let cmd = cmd_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap();
        assert!(matches!(
            cmd,
            scylla_core::messenger::AppCommand::RetryAllFailed
        ));
    }

    #[tokio::test]
    async fn test_flush_completed_sends_command_and_clears_snapshot() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let db = Arc::new(Mutex::new(db::ServerDb::open_conn(conn).unwrap()));
        let (cmd_tx, cmd_rx) = std::sync::mpsc::channel();
        let registry = Arc::new(std::sync::Mutex::new(
            scylla_core::scraper::ScraperRegistry::new(),
        ));
        let jobs = Arc::new(std::sync::Mutex::new(vec![
            scylla_core::types::JobDto {
                id: 1,
                kind: "Scrape".into(),
                status: "Completed".into(),
                target: "http://example.com".into(),
                priority: "Normal".into(),
                chapter_idx: None,
                created_at_ms: 0,
                started_at_ms: None,
                completed_at_ms: None,
                error: None,
                outcome: None,
                detail: None,
            },
            scylla_core::types::JobDto {
                id: 2,
                kind: "Scrape".into(),
                status: "Running".into(),
                target: "http://example.com/2".into(),
                priority: "Normal".into(),
                chapter_idx: None,
                created_at_ms: 0,
                started_at_ms: None,
                completed_at_ms: None,
                error: None,
                outcome: None,
                detail: None,
            },
        ]));
        let state =
            Arc::new(AppState {
                db,
                cmd_tx,
                registry,
                max_workers: std::sync::Mutex::new(4),
                rate_limit: std::sync::Mutex::new(2),
                jobs,
                job_events:
                    tokio::sync::broadcast::channel::<Arc<scylla_core::messenger::AppEvent>>(256).0,
                embedder: Arc::new(embeddings::SharedEmbedder::new()),
                reranker: Arc::new(embeddings::SharedCrossEncoder::in_cooldown()),
                autoembed: Arc::new(std::sync::Mutex::new(false)),
            });
        let app = build_router(state);

        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/jobs/flush-completed")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let cmd = cmd_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap();
        assert!(matches!(
            cmd,
            scylla_core::messenger::AppCommand::FlushCompleted
        ));

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/jobs")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let json = body_json(resp).await;
        let list = json["jobs"].as_array().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0]["id"], 2);
    }

    #[tokio::test]
    async fn test_flush_all_sends_command_and_clears_snapshot() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let db = Arc::new(Mutex::new(db::ServerDb::open_conn(conn).unwrap()));
        let (cmd_tx, cmd_rx) = std::sync::mpsc::channel();
        let registry = Arc::new(std::sync::Mutex::new(
            scylla_core::scraper::ScraperRegistry::new(),
        ));
        let jobs = Arc::new(std::sync::Mutex::new(vec![scylla_core::types::JobDto {
            id: 1,
            kind: "Scrape".into(),
            status: "Completed".into(),
            target: "http://example.com".into(),
            priority: "Normal".into(),
            chapter_idx: None,
            created_at_ms: 0,
            started_at_ms: None,
            completed_at_ms: None,
            error: None,
            outcome: None,
            detail: None,
        }]));
        let state =
            Arc::new(AppState {
                db,
                cmd_tx,
                registry,
                max_workers: std::sync::Mutex::new(4),
                rate_limit: std::sync::Mutex::new(2),
                jobs,
                job_events:
                    tokio::sync::broadcast::channel::<Arc<scylla_core::messenger::AppEvent>>(256).0,
                embedder: Arc::new(embeddings::SharedEmbedder::new()),
                reranker: Arc::new(embeddings::SharedCrossEncoder::in_cooldown()),
                autoembed: Arc::new(std::sync::Mutex::new(false)),
            });
        let app = build_router(state);

        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/jobs/flush-all")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let cmd = cmd_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap();
        assert!(matches!(cmd, scylla_core::messenger::AppCommand::FlushAll));

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/jobs")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let json = body_json(resp).await;
        assert_eq!(json["jobs"].as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn test_set_workers_sends_command_and_updates_state() {
        let (app, cmd_rx) = test_app_with_cmd_rx();
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/jobs/workers")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"max_workers":6}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let cmd = cmd_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap();
        assert!(matches!(
            cmd,
            scylla_core::messenger::AppCommand::SetMaxWorkers(6)
        ));

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/settings")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let json = body_json(resp).await;
        assert_eq!(json["max_workers"], 6);
    }

    #[tokio::test]
    async fn test_set_rate_limit_sends_command_and_updates_state() {
        let (app, cmd_rx) = test_app_with_cmd_rx();
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/jobs/rate-limit")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"rate_limit":5}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let cmd = cmd_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap();
        assert!(matches!(
            cmd,
            scylla_core::messenger::AppCommand::SetRateLimit(5)
        ));

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/settings")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let json = body_json(resp).await;
        assert_eq!(json["rate_limit"], 5);
    }

    #[tokio::test]
    async fn test_job_command_bad_body_returns_400() {
        let (app, _cmd_rx) = test_app_with_cmd_rx();
        for uri in ["/api/jobs/cancel", "/api/jobs/retry"] {
            let resp = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri(uri)
                        .header("content-type", "application/json")
                        .body(Body::from(r#"{}"#))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        }
    }

    #[tokio::test]
    async fn test_enqueue_scrape_sends_command() {
        let (app, cmd_rx) = test_app_with_cmd_rx();
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/jobs/enqueue")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"kind":"Scrape","url":"http://example.com/book"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::ACCEPTED);
        let cmd = cmd_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap();
        assert!(matches!(
            cmd,
            scylla_core::messenger::AppCommand::Scrape(url) if url == "http://example.com/book"
        ));
    }

    #[tokio::test]
    async fn test_enqueue_fetch_chapter_sends_command() {
        let (app, cmd_rx) = test_app_with_cmd_rx();
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/jobs/enqueue")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"kind":"FetchChapter","url":"http://example.com/ch1","chapter_idx":3}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::ACCEPTED);
        let cmd = cmd_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap();
        assert!(matches!(
            cmd,
            scylla_core::messenger::AppCommand::FetchChapter(url, 3)
                if url == "http://example.com/ch1"
        ));
    }

    #[tokio::test]
    async fn test_enqueue_fetch_cover_sends_command() {
        let (app, cmd_rx) = test_app_with_cmd_rx();
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/jobs/enqueue")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"kind":"FetchCover","url":"http://example.com/cover.jpg"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::ACCEPTED);
        let cmd = cmd_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap();
        assert!(matches!(
            cmd,
            scylla_core::messenger::AppCommand::FetchCover(url)
                if url == "http://example.com/cover.jpg"
        ));
    }

    #[tokio::test]
    async fn test_enqueue_bad_requests_return_400() {
        let (app, _cmd_rx) = test_app_with_cmd_rx();
        let cases = [
            r#"{"kind":"Unknown","url":"http://example.com"}"#,
            r#"{"kind":"FetchChapter","url":"http://example.com/ch1"}"#,
            r#"{"kind":"Scrape"}"#,
            r#"{"kind":"Scrape","url":""}"#,
            r#"{"kind":"EmbedBatch"}"#,
            r#"{"kind":"EmbedBatch","chapters":[]}"#,
            r#"{"kind":"EmbedBatch","chapters":[{"idx":0,"title":"Ch1"}]}"#,
        ];
        for body in cases {
            let resp = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/api/jobs/enqueue")
                        .header("content-type", "application/json")
                        .body(Body::from(body))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "body: {}", body);
        }
    }

    #[tokio::test]
    async fn test_enqueue_embed_batch_sends_command() {
        let (app, cmd_rx) = test_app_with_cmd_rx();
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/jobs/enqueue")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"kind":"EmbedBatch","chapters":[{"url":"http://example.com/ch1","idx":0,"title":"Ch 1"},{"url":"http://example.com/ch2","idx":1,"title":"Ch 2"}]}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::ACCEPTED);
        let cmd = cmd_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap();
        match cmd {
            scylla_core::messenger::AppCommand::EmbedBatch(chapters) => {
                assert_eq!(chapters.len(), 2);
                assert_eq!(chapters[0].url, "http://example.com/ch1");
                assert_eq!(chapters[0].idx, 0);
                assert_eq!(chapters[0].title, "Ch 1");
                assert_eq!(chapters[1].url, "http://example.com/ch2");
            }
            _ => panic!("expected EmbedBatch command"),
        }
    }

    #[tokio::test]
    async fn test_enqueue_install_plugin_sends_command() {
        let (app, cmd_rx) = test_app_with_cmd_rx();
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/jobs/enqueue")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"kind":"InstallPlugin","url":"https://github.com/o/r"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::ACCEPTED);
        let cmd = cmd_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap();
        assert!(matches!(
            cmd,
            scylla_core::messenger::AppCommand::InstallPlugin(url)
                if url == "https://github.com/o/r"
        ));
    }

    #[tokio::test]
    async fn test_install_plugin_sends_command() {
        let (app, cmd_rx) = test_app_with_cmd_rx();
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/plugins/install")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"repo_url":"https://github.com/o/r"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::ACCEPTED);
        let cmd = cmd_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap();
        assert!(matches!(
            cmd,
            scylla_core::messenger::AppCommand::InstallPlugin(url)
                if url == "https://github.com/o/r"
        ));
    }

    #[tokio::test]
    async fn test_install_plugin_bad_body_returns_400() {
        let (app, _cmd_rx) = test_app_with_cmd_rx();
        for body in [r#"{}"#, r#"{"repo_url":""}"#] {
            let resp = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/api/plugins/install")
                        .header("content-type", "application/json")
                        .body(Body::from(body))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        }
    }

    #[tokio::test]
    async fn test_crawl_book_sends_fetch_chapter_commands() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let db = Arc::new(Mutex::new(db::ServerDb::open_conn(conn).unwrap()));
        let (cmd_tx, cmd_rx) = std::sync::mpsc::channel();
        let registry = Arc::new(std::sync::Mutex::new(
            scylla_core::scraper::ScraperRegistry::new(),
        ));
        let state =
            Arc::new(AppState {
                db,
                cmd_tx,
                registry,
                max_workers: std::sync::Mutex::new(4),
                rate_limit: std::sync::Mutex::new(2),
                jobs: Arc::new(std::sync::Mutex::new(Vec::new())),
                job_events:
                    tokio::sync::broadcast::channel::<Arc<scylla_core::messenger::AppEvent>>(256).0,
                embedder: Arc::new(embeddings::SharedEmbedder::new()),
                reranker: Arc::new(embeddings::SharedCrossEncoder::in_cooldown()),
                autoembed: Arc::new(std::sync::Mutex::new(false)),
            });
        let app = build_router(state);

        // Seed a book with two chapters.
        let book = scylla_core::types::Book {
            title: "Book".into(),
            url: "http://example.com/book".into(),
            status: scylla_core::types::BookStatus::Reading,
            sessions: vec![],
            active_session_id: None,
            tags: vec![],
            cover_url: None,
            description: None,
            chapters: vec![
                scylla_core::types::Chapter {
                    url: "http://example.com/ch1".into(),
                    title: "Ch1".into(),
                    order: 1,
                },
                scylla_core::types::Chapter {
                    url: "http://example.com/ch2".into(),
                    title: "Ch2".into(),
                    order: 2,
                },
            ],
        };
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/books/http%3A%2F%2Fexample.com%2Fbook")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_string(&book).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/books/http%3A%2F%2Fexample.com%2Fbook/crawl")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::ACCEPTED);

        let c1 = cmd_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap();
        let c2 = cmd_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap();
        assert!(matches!(
            c1,
            scylla_core::messenger::AppCommand::FetchChapter(url, 1)
                if url == "http://example.com/ch1"
        ));
        assert!(matches!(
            c2,
            scylla_core::messenger::AppCommand::FetchChapter(url, 2)
                if url == "http://example.com/ch2"
        ));
    }

    #[tokio::test]
    async fn test_crawl_missing_book_returns_404() {
        let (app, _cmd_rx) = test_app_with_cmd_rx();
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/books/http%3A%2F%2Fexample.com%2Fnope/crawl")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn test_should_recompute() {
        assert!(should_recompute(1));
        assert!(!should_recompute(2));
        assert!(!should_recompute(4));
        assert!(should_recompute(5));
        assert!(!should_recompute(6));
        assert!(should_recompute(10));
        assert!(should_recompute(15));
    }

    #[test]
    fn test_content_hash_stable_and_distinct() {
        let h1 = content_hash("same text");
        let h2 = content_hash("same text");
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 16);
        assert_ne!(h1, content_hash("different text"));
        assert_ne!(content_hash(""), content_hash(" "));
    }

    #[test]
    fn test_content_hash_includes_model_version() {
        // FNV-1a over MODEL_VERSION + text: a model swap must change every hash
        // so unchanged chapters are re-embedded exactly once.
        let hash = content_hash("chapter text");
        let mut expected = 0xcbf29ce484222325u64;
        for byte in embeddings::MODEL_VERSION
            .as_bytes()
            .iter()
            .chain("chapter text".as_bytes())
        {
            expected ^= *byte as u64;
            expected = expected.wrapping_mul(0x100000001b3);
        }
        assert_eq!(hash, format!("{:016x}", expected));
    }

    fn stub_embed(vec: Vec<f32>) -> embeddings::EmbedFn {
        Arc::new(move |texts: &[&str], _mode: embeddings::EmbedMode| {
            Ok(vec![vec.clone(); texts.len()])
        })
    }

    /// Stub reranker: scores each passage by its length (longer = more
    /// relevant), so rerank order is deterministic and testable.
    fn stub_rerank() -> embeddings::RerankFn {
        Arc::new(|_query: &str, passages: &[&str]| {
            Ok(passages.iter().map(|p| p.len() as f32).collect())
        })
    }

    #[tokio::test]
    async fn test_search_book_mode_ranks_by_similarity() {
        let (app, db) = test_app_with_embed_and_db(stub_embed(vec![1.0, 0.0]));
        // Seed books so titles resolve, aggregates for ranking, and one chapter
        // embedding so the corpus is non-empty (book mode needs it to rank).
        let book_a = scylla_core::types::Book {
            title: "Book A".into(),
            url: "book-a".into(),
            status: scylla_core::types::BookStatus::Reading,
            sessions: vec![],
            active_session_id: None,
            tags: vec![],
            cover_url: None,
            description: None,
            chapters: vec![scylla_core::types::Chapter {
                url: "ch-a1".into(),
                title: "Chapter A1".into(),
                order: 1,
            }],
        };
        let book_b = scylla_core::types::Book {
            title: "Book B".into(),
            url: "book-b".into(),
            status: scylla_core::types::BookStatus::Reading,
            sessions: vec![],
            active_session_id: None,
            tags: vec![],
            cover_url: None,
            description: None,
            chapters: vec![],
        };
        db.lock().await.upsert_book(&book_a).unwrap();
        db.lock().await.upsert_book(&book_b).unwrap();
        db.lock()
            .await
            .upsert_book_embedding(
                "book-a",
                None,
                Some(&[1.0, 0.0]),
                Some(&["Fantasy".to_string()]),
            )
            .unwrap();
        db.lock()
            .await
            .upsert_book_embedding("book-b", None, Some(&[0.0, 1.0]), None)
            .unwrap();
        db.lock()
            .await
            .upsert_chapter_chunks("ch-a1", Some("book-a"), &[(0, "chunk", &[1.0, 0.0])], None)
            .unwrap();

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/search")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"query":"x","mode":"book","limit":10}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp).await;
        assert_eq!(json["mode"], "book");
        let hits = json["hits"].as_array().unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0]["book_url"], "book-a");
        assert_eq!(hits[0]["title"], "Book A");
        assert_eq!(hits[0]["genres"][0], "Fantasy");
        assert_eq!(hits[1]["book_url"], "book-b");
        assert_eq!(hits[1]["title"], "Book B");
        assert!(hits[0]["score"].as_f64().unwrap() > hits[1]["score"].as_f64().unwrap());
        // Book A has one inline chapter; Book B has none.
        assert_eq!(hits[0]["chapters"].as_array().unwrap().len(), 1);
        assert_eq!(hits[1]["chapters"].as_array().unwrap().len(), 0);
        // Coverage fields: 1 of 1 chapters embedded.
        assert_eq!(json["embedded"], 1);
        assert_eq!(json["total"], 1);
    }

    #[tokio::test]
    async fn test_search_chapter_mode_flat_hits() {
        let (app, db) = test_app_with_embed_and_db(stub_embed(vec![1.0, 0.0]));
        // Seed books with chapters so the join enriches the hits.
        let book_a = scylla_core::types::Book {
            title: "Book A".into(),
            url: "book-a".into(),
            status: scylla_core::types::BookStatus::Reading,
            sessions: vec![],
            active_session_id: None,
            tags: vec![],
            cover_url: None,
            description: None,
            chapters: vec![
                scylla_core::types::Chapter {
                    url: "ch-a1".into(),
                    title: "Chapter A1".into(),
                    order: 1,
                },
                scylla_core::types::Chapter {
                    url: "ch-a2".into(),
                    title: "Chapter A2".into(),
                    order: 2,
                },
            ],
        };
        let book_b = scylla_core::types::Book {
            title: "Book B".into(),
            url: "book-b".into(),
            status: scylla_core::types::BookStatus::Reading,
            sessions: vec![],
            active_session_id: None,
            tags: vec![],
            cover_url: None,
            description: None,
            chapters: vec![scylla_core::types::Chapter {
                url: "ch-b1".into(),
                title: "Chapter B1".into(),
                order: 1,
            }],
        };
        db.lock().await.upsert_book(&book_a).unwrap();
        db.lock().await.upsert_book(&book_b).unwrap();
        db.lock()
            .await
            .upsert_book_embedding(
                "book-a",
                None,
                Some(&[1.0, 0.0]),
                Some(&["Fantasy".to_string()]),
            )
            .unwrap();
        db.lock()
            .await
            .upsert_chapter_chunks("ch-a1", Some("book-a"), &[(0, "chunk", &[1.0, 0.0])], None)
            .unwrap();
        db.lock()
            .await
            .upsert_chapter_chunks("ch-a2", Some("book-a"), &[(0, "chunk", &[0.9, 0.1])], None)
            .unwrap();
        db.lock()
            .await
            .upsert_chapter_chunks("ch-b1", Some("book-b"), &[(0, "chunk", &[0.5, 0.5])], None)
            .unwrap();

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/search")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"query":"x","mode":"chapter","limit":10}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp).await;
        assert_eq!(json["mode"], "chapter");
        let hits = json["hits"].as_array().unwrap();
        assert_eq!(hits.len(), 3);
        // Flat, sorted by score desc. Wire shape is the pinned contract:
        // {"url","chapter_idx","title","score"}.
        assert_eq!(hits[0]["url"], "ch-a1");
        assert_eq!(hits[0]["title"], "Chapter A1");
        assert_eq!(hits[0]["chapter_idx"], 1);
        assert_eq!(hits[1]["url"], "ch-a2");
        assert_eq!(hits[2]["url"], "ch-b1");
        assert_eq!(hits[2]["title"], "Chapter B1");
        // Coverage fields: all 3 seeded chapters are embedded.
        assert_eq!(json["embedded"], 3);
        assert_eq!(json["total"], 3);
    }

    #[tokio::test]
    async fn test_search_chapter_mode_reranks_by_cross_encoder() {
        // Stub reranker scores by text length, so the longest chunk text must
        // surface first even though the bi-encoder sees identical embeddings.
        let (app, db) =
            test_app_with_embed_rerank_and_db(stub_embed(vec![1.0, 0.0]), stub_rerank());
        let book = scylla_core::types::Book {
            title: "Book A".into(),
            url: "book-a".into(),
            status: scylla_core::types::BookStatus::Reading,
            sessions: vec![],
            active_session_id: None,
            tags: vec![],
            cover_url: None,
            description: None,
            chapters: vec![
                scylla_core::types::Chapter {
                    url: "ch-short".into(),
                    title: "Short".into(),
                    order: 1,
                },
                scylla_core::types::Chapter {
                    url: "ch-long".into(),
                    title: "Long".into(),
                    order: 2,
                },
            ],
        };
        db.lock().await.upsert_book(&book).unwrap();
        db.lock()
            .await
            .upsert_chapter_chunks(
                "ch-short",
                Some("book-a"),
                &[(0, "short", &[1.0, 0.0])],
                None,
            )
            .unwrap();
        db.lock()
            .await
            .upsert_chapter_chunks(
                "ch-long",
                Some("book-a"),
                &[(0, "a much longer chunk text", &[1.0, 0.0])],
                None,
            )
            .unwrap();

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/search")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"query":"x","mode":"chapter","limit":10}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp).await;
        let hits = json["hits"].as_array().unwrap();
        assert_eq!(hits.len(), 2);
        // Reranked: the longer chunk text wins despite equal bi-encoder scores.
        assert_eq!(hits[0]["url"], "ch-long");
        assert_eq!(hits[1]["url"], "ch-short");
    }

    #[tokio::test]
    async fn test_search_book_mode_reranks_by_cross_encoder() {
        // Two books with identical aggregate embeddings; the stub reranker
        // prefers the book whose best chapter chunk text is longer.
        let (app, db) =
            test_app_with_embed_rerank_and_db(stub_embed(vec![1.0, 0.0]), stub_rerank());
        for (url, title, chunk_text) in [
            ("book-a", "Book A", "short"),
            ("book-b", "Book B", "a much longer chunk text"),
        ] {
            let book = scylla_core::types::Book {
                title: title.into(),
                url: url.into(),
                status: scylla_core::types::BookStatus::Reading,
                sessions: vec![],
                active_session_id: None,
                tags: vec![],
                cover_url: None,
                description: None,
                chapters: vec![scylla_core::types::Chapter {
                    url: format!("ch-{url}"),
                    title: format!("Chapter {url}"),
                    order: 1,
                }],
            };
            db.lock().await.upsert_book(&book).unwrap();
            db.lock()
                .await
                .upsert_book_embedding(url, None, Some(&[1.0, 0.0]), None)
                .unwrap();
            db.lock()
                .await
                .upsert_chapter_chunks(
                    &format!("ch-{url}"),
                    Some(url),
                    &[(0, chunk_text, &[1.0, 0.0])],
                    None,
                )
                .unwrap();
        }

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/search")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"query":"x","mode":"book","limit":10}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp).await;
        let hits = json["hits"].as_array().unwrap();
        assert_eq!(hits.len(), 2);
        // Reranked: book-b's longer best-chunk text wins over the tie.
        assert_eq!(hits[0]["book_url"], "book-b");
        assert_eq!(hits[1]["book_url"], "book-a");
    }

    #[tokio::test]
    async fn test_search_chapter_mode_bm25_surfaces_below_floor_chapters() {
        // The stub embedder embeds the query as [1.0, 0.0]. One chapter is
        // semantically relevant (cosine 1.0); the other is orthogonal to the
        // query (cosine 0.0 — below the floor, so the semantic lane drops it)
        // but its text matches the query via BM25. The BM25 lane must surface it.
        let (app, db) = test_app_with_embed_and_db(stub_embed(vec![1.0, 0.0]));
        let book = scylla_core::types::Book {
            title: "Book A".into(),
            url: "book-a".into(),
            status: scylla_core::types::BookStatus::Reading,
            sessions: vec![],
            active_session_id: None,
            tags: vec![],
            cover_url: None,
            description: None,
            chapters: vec![
                scylla_core::types::Chapter {
                    url: "ch-a1".into(),
                    title: "Chapter A1".into(),
                    order: 1,
                },
                scylla_core::types::Chapter {
                    url: "ch-b1".into(),
                    title: "Chapter B1".into(),
                    order: 2,
                },
            ],
        };
        db.lock().await.upsert_book(&book).unwrap();
        db.lock()
            .await
            .upsert_chapter_chunks(
                "ch-a1",
                Some("book-a"),
                &[(0, "unrelated filler text", &[1.0, 0.0])],
                None,
            )
            .unwrap();
        db.lock()
            .await
            .upsert_chapter_chunks(
                "ch-b1",
                Some("book-a"),
                &[(0, "the dragon sleeps in the cave", &[0.0, 1.0])],
                None,
            )
            .unwrap();

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/search")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"query":"dragon","mode":"chapter","limit":10}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp).await;
        let hits = json["hits"].as_array().unwrap();
        let urls: Vec<&str> = hits.iter().map(|h| h["url"].as_str().unwrap()).collect();
        // ch-b1 appears despite cosine 0.0 (below floor) — the BM25 lane.
        assert!(urls.contains(&"ch-b1"), "hits: {urls:?}");
        // ch-a1 (semantic hit) is present too.
        assert!(urls.contains(&"ch-a1"), "hits: {urls:?}");
    }

    #[tokio::test]
    async fn test_search_chapter_mode_caps_dominant_book_post_fusion() {
        // Book A has 4 above-floor chapters whose text all matches the query;
        // Book B has 1 above-floor chapter that does not. Without the post-fusion
        // cap, book A's 4 chapters would occupy the top-4. With it, book B's
        // chapter takes a top slot and book A's 4th chapter is pushed down.
        let (app, db) = test_app_with_embed_and_db(stub_embed(vec![1.0, 0.0]));
        let book_a = scylla_core::types::Book {
            title: "Book A".into(),
            url: "book-a".into(),
            status: scylla_core::types::BookStatus::Reading,
            sessions: vec![],
            active_session_id: None,
            tags: vec![],
            cover_url: None,
            description: None,
            chapters: (1..=4)
                .map(|i| scylla_core::types::Chapter {
                    url: format!("ch-a{}", i),
                    title: format!("Chapter A{}", i),
                    order: i,
                })
                .collect(),
        };
        let book_b = scylla_core::types::Book {
            title: "Book B".into(),
            url: "book-b".into(),
            status: scylla_core::types::BookStatus::Reading,
            sessions: vec![],
            active_session_id: None,
            tags: vec![],
            cover_url: None,
            description: None,
            chapters: vec![scylla_core::types::Chapter {
                url: "ch-b1".into(),
                title: "Chapter B1".into(),
                order: 1,
            }],
        };
        db.lock().await.upsert_book(&book_a).unwrap();
        db.lock().await.upsert_book(&book_b).unwrap();
        for (emb, url) in [
            ([1.0, 0.0], "ch-a1"),
            ([0.9, 0.1], "ch-a2"),
            ([0.8, 0.2], "ch-a3"),
            ([0.7, 0.3], "ch-a4"),
        ] {
            db.lock()
                .await
                .upsert_chapter_chunks(url, Some("book-a"), &[(0, "dragon", &emb)], None)
                .unwrap();
        }
        db.lock()
            .await
            .upsert_chapter_chunks(
                "ch-b1",
                Some("book-b"),
                &[(0, "unrelated", &[1.0, 0.0])],
                None,
            )
            .unwrap();

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/search")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"query":"dragon","mode":"chapter","limit":4}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp).await;
        let hits = json["hits"].as_array().unwrap();
        assert_eq!(hits.len(), 4);
        // Book B's chapter is in the top-4 (the cap reserved a slot for it),
        // and book A's 4th chapter (ch-a4) is pushed out of the top-4.
        let urls: Vec<&str> = hits.iter().map(|h| h["url"].as_str().unwrap()).collect();
        assert!(urls.contains(&"ch-b1"), "hits: {urls:?}");
        assert!(!urls.contains(&"ch-a4"), "hits: {urls:?}");
    }

    #[tokio::test]
    async fn test_search_chapter_mode_book_url_filters_and_no_cap() {
        let (app, db) = test_app_with_embed_and_db(stub_embed(vec![1.0, 0.0]));
        // Book A with 5 chapters (all above the floor), Book B with 1.
        let book_a = scylla_core::types::Book {
            title: "Book A".into(),
            url: "book-a".into(),
            status: scylla_core::types::BookStatus::Reading,
            sessions: vec![],
            active_session_id: None,
            tags: vec![],
            cover_url: None,
            description: None,
            chapters: (1..=5)
                .map(|i| scylla_core::types::Chapter {
                    url: format!("ch-a{}", i),
                    title: format!("Chapter A{}", i),
                    order: i,
                })
                .collect(),
        };
        let book_b = scylla_core::types::Book {
            title: "Book B".into(),
            url: "book-b".into(),
            status: scylla_core::types::BookStatus::Reading,
            sessions: vec![],
            active_session_id: None,
            tags: vec![],
            cover_url: None,
            description: None,
            chapters: vec![scylla_core::types::Chapter {
                url: "ch-b1".into(),
                title: "Chapter B1".into(),
                order: 1,
            }],
        };
        db.lock().await.upsert_book(&book_a).unwrap();
        db.lock().await.upsert_book(&book_b).unwrap();
        for emb in [
            ([1.0, 0.0], "ch-a1"),
            ([0.9, 0.1], "ch-a2"),
            ([0.8, 0.2], "ch-a3"),
            ([0.7, 0.3], "ch-a4"),
            ([0.6, 0.4], "ch-a5"),
        ] {
            db.lock()
                .await
                .upsert_chapter_chunks(emb.1, Some("book-a"), &[(0, "chunk", &emb.0)], None)
                .unwrap();
        }
        db.lock()
            .await
            .upsert_chapter_chunks("ch-b1", Some("book-b"), &[(0, "chunk", &[0.5, 0.5])], None)
            .unwrap();

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/search")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"query":"x","mode":"chapter","limit":50,"book_url":"book-a"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp).await;
        assert_eq!(json["mode"], "chapter");
        let hits = json["hits"].as_array().unwrap();
        // All 5 of book A's chapters come back — the per-book diversity cap is
        // a no-op for the single-book drill-down window.
        assert_eq!(hits.len(), 5);
        for h in hits {
            assert!(h["url"].as_str().unwrap().starts_with("ch-a"));
        }
        // Sorted by score desc.
        assert_eq!(hits[0]["url"], "ch-a1");
        assert_eq!(hits[4]["url"], "ch-a5");
        // Coverage stays library-level.
        assert_eq!(json["embedded"], 6);
        assert_eq!(json["total"], 6);
    }

    #[tokio::test]
    async fn test_search_book_mode_inline_chapters_carry_snippets() {
        let (app, db) = test_app_with_embed_and_db(stub_embed(vec![1.0, 0.0]));
        let book = scylla_core::types::Book {
            title: "Book A".into(),
            url: "book-a".into(),
            status: scylla_core::types::BookStatus::Reading,
            sessions: vec![],
            active_session_id: None,
            tags: vec![],
            cover_url: None,
            description: None,
            chapters: vec![scylla_core::types::Chapter {
                url: "ch-a1".into(),
                title: "Chapter A1".into(),
                order: 1,
            }],
        };
        db.lock().await.upsert_book(&book).unwrap();
        db.lock()
            .await
            .upsert_book_embedding("book-a", None, Some(&[1.0, 0.0]), None)
            .unwrap();
        db.lock()
            .await
            .upsert_chapter_chunks(
                "ch-a1",
                Some("book-a"),
                &[(
                    0,
                    "the dragon sleeps beneath the mountain for a thousand years",
                    &[1.0, 0.0],
                )],
                None,
            )
            .unwrap();

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/search")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"query":"dragon","mode":"book","limit":10}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp).await;
        let hits = json["hits"].as_array().unwrap();
        assert_eq!(hits.len(), 1);
        let chapters = hits[0]["chapters"].as_array().unwrap();
        assert_eq!(chapters.len(), 1);
        let snippet = chapters[0]["snippet"].as_str().unwrap();
        assert!(snippet.contains("dragon"), "snippet: {snippet}");
    }

    #[tokio::test]
    async fn test_search_book_mode_inline_chapters() {
        let (app, db) = test_app_with_embed_and_db(stub_embed(vec![1.0, 0.0]));
        // Book A (4 chapters) and Book B (3 chapters), all above the floor.
        let book_a = scylla_core::types::Book {
            title: "Book A".into(),
            url: "book-a".into(),
            status: scylla_core::types::BookStatus::Reading,
            sessions: vec![],
            active_session_id: None,
            tags: vec![],
            cover_url: None,
            description: None,
            chapters: (1..=4)
                .map(|i| scylla_core::types::Chapter {
                    url: format!("ch-a{}", i),
                    title: format!("Chapter A{}", i),
                    order: i,
                })
                .collect(),
        };
        let book_b = scylla_core::types::Book {
            title: "Book B".into(),
            url: "book-b".into(),
            status: scylla_core::types::BookStatus::Reading,
            sessions: vec![],
            active_session_id: None,
            tags: vec![],
            cover_url: None,
            description: None,
            chapters: (1..=3)
                .map(|i| scylla_core::types::Chapter {
                    url: format!("ch-b{}", i),
                    title: format!("Chapter B{}", i),
                    order: i,
                })
                .collect(),
        };
        db.lock().await.upsert_book(&book_a).unwrap();
        db.lock().await.upsert_book(&book_b).unwrap();
        db.lock()
            .await
            .upsert_book_embedding(
                "book-a",
                None,
                Some(&[1.0, 0.0]),
                Some(&["Fantasy".to_string()]),
            )
            .unwrap();
        db.lock()
            .await
            .upsert_book_embedding("book-b", None, Some(&[0.5, 0.5]), None)
            .unwrap();
        for emb in [
            ([1.0, 0.0], "ch-a1"),
            ([0.9, 0.1], "ch-a2"),
            ([0.8, 0.2], "ch-a3"),
            ([0.7, 0.3], "ch-a4"),
        ] {
            db.lock()
                .await
                .upsert_chapter_chunks(emb.1, Some("book-a"), &[(0, "chunk", &emb.0)], None)
                .unwrap();
        }
        for emb in [
            ([0.5, 0.5], "ch-b1"),
            ([0.4, 0.6], "ch-b2"),
            ([0.3, 0.7], "ch-b3"),
        ] {
            db.lock()
                .await
                .upsert_chapter_chunks(emb.1, Some("book-b"), &[(0, "chunk", &emb.0)], None)
                .unwrap();
        }

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/search")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"query":"x","mode":"book","limit":10}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp).await;
        assert_eq!(json["mode"], "book");
        let hits = json["hits"].as_array().unwrap();
        assert_eq!(hits.len(), 2);
        // Book A first (higher aggregate score), with its top-3 inline.
        assert_eq!(hits[0]["book_url"], "book-a");
        assert_eq!(hits[0]["title"], "Book A");
        assert_eq!(hits[0]["score"].as_f64().unwrap(), 100.0);
        assert_eq!(hits[0]["genres"][0], "Fantasy");
        let a_chapters = hits[0]["chapters"].as_array().unwrap();
        assert_eq!(a_chapters.len(), 3); // top-3, not all 4
        assert_eq!(a_chapters[0]["url"], "ch-a1");
        assert_eq!(a_chapters[0]["chapter_idx"], 1);
        assert_eq!(a_chapters[0]["title"], "Chapter A1");
        assert_eq!(a_chapters[0]["score"].as_f64().unwrap(), 100.0);
        assert_eq!(a_chapters[1]["url"], "ch-a2");
        assert_eq!(a_chapters[2]["url"], "ch-a3");
        // Book B second, with its top-3.
        assert_eq!(hits[1]["book_url"], "book-b");
        assert_eq!(hits[1]["title"], "Book B");
        let b_chapters = hits[1]["chapters"].as_array().unwrap();
        assert_eq!(b_chapters.len(), 3);
        assert_eq!(b_chapters[0]["url"], "ch-b1");
        assert_eq!(b_chapters[2]["url"], "ch-b3");
        // Coverage: all 7 chapters embedded.
        assert_eq!(json["embedded"], 7);
        assert_eq!(json["total"], 7);
    }

    #[tokio::test]
    async fn test_search_empty_query_returns_400() {
        let app = test_app();
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/search")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"query":"","mode":"book"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_search_unknown_mode_returns_400() {
        let app = test_app();
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/search")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"query":"x","mode":"bogus"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_search_limit_clamped() {
        let (app, db) = test_app_with_embed_and_db(stub_embed(vec![1.0, 0.0]));
        db.lock()
            .await
            .upsert_book_embedding("book-a", None, Some(&[1.0, 0.0]), None)
            .unwrap();
        db.lock()
            .await
            .upsert_book_embedding("book-b", None, Some(&[0.0, 1.0]), None)
            .unwrap();
        // Seed one chapter embedding so the corpus is non-empty (book mode
        // returns no_embeddings on an empty corpus before ranking).
        db.lock()
            .await
            .upsert_chapter_chunks("ch-a1", Some("book-a"), &[(0, "chunk", &[1.0, 0.0])], None)
            .unwrap();

        // limit 0 clamps to 1; limit 999 clamps to 50 (only 2 books exist).
        for (limit, expected) in [("0", 1), ("999", 2)] {
            let resp = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/api/search")
                        .header("content-type", "application/json")
                        .body(Body::from(format!(
                            r#"{{"query":"x","mode":"book","limit":{}}}"#,
                            limit
                        )))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::OK);
            let json = body_json(resp).await;
            assert_eq!(json["hits"].as_array().unwrap().len(), expected);
        }
    }

    #[tokio::test]
    async fn test_search_returns_503_when_embedder_unavailable() {
        // A non-empty corpus reaches the embedder; an offline first run then
        // gets a 503 (the empty-corpus no_embeddings hint is checked first).
        let (app, db) = test_app_with_embedder_in_cooldown_and_db();
        let book = scylla_core::types::Book {
            title: "Book A".into(),
            url: "book-a".into(),
            status: scylla_core::types::BookStatus::Reading,
            sessions: vec![],
            active_session_id: None,
            tags: vec![],
            cover_url: None,
            description: None,
            chapters: vec![scylla_core::types::Chapter {
                url: "ch-a1".into(),
                title: "Chapter A1".into(),
                order: 1,
            }],
        };
        db.lock().await.upsert_book(&book).unwrap();
        db.lock()
            .await
            .upsert_chapter_chunks("ch-a1", Some("book-a"), &[(0, "chunk", &[1.0, 0.0])], None)
            .unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/search")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"query":"x","mode":"book"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn test_search_chapter_mode_no_embeddings_returns_hint() {
        let (app, db) = test_app_with_embed_and_db(stub_embed(vec![1.0, 0.0]));
        // Seed a book with chapters but no chapter embeddings.
        let book = scylla_core::types::Book {
            title: "Book A".into(),
            url: "book-a".into(),
            status: scylla_core::types::BookStatus::Reading,
            sessions: vec![],
            active_session_id: None,
            tags: vec![],
            cover_url: None,
            description: None,
            chapters: vec![
                scylla_core::types::Chapter {
                    url: "ch-a1".into(),
                    title: "Chapter A1".into(),
                    order: 1,
                },
                scylla_core::types::Chapter {
                    url: "ch-a2".into(),
                    title: "Chapter A2".into(),
                    order: 2,
                },
            ],
        };
        db.lock().await.upsert_book(&book).unwrap();

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/search")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"query":"x","mode":"chapter","limit":10}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp).await;
        assert_eq!(json["error"], "no_embeddings");
        assert_eq!(json["embedded"], 0);
        assert_eq!(json["total"], 2);
    }

    #[tokio::test]
    async fn test_search_chapter_mode_no_embeddings_beats_embedder_503() {
        // Offline first run: the embedder is in cooldown, but an empty corpus
        // must still produce the hint, not a generic 503.
        let (app, db) = test_app_with_embedder_in_cooldown_and_db();
        let book = scylla_core::types::Book {
            title: "Book A".into(),
            url: "book-a".into(),
            status: scylla_core::types::BookStatus::Reading,
            sessions: vec![],
            active_session_id: None,
            tags: vec![],
            cover_url: None,
            description: None,
            chapters: vec![scylla_core::types::Chapter {
                url: "ch-a1".into(),
                title: "Chapter A1".into(),
                order: 1,
            }],
        };
        db.lock().await.upsert_book(&book).unwrap();

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/search")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"query":"x","mode":"chapter","limit":10}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp).await;
        assert_eq!(json["error"], "no_embeddings");
        assert_eq!(json["embedded"], 0);
        assert_eq!(json["total"], 1);
    }

    #[tokio::test]
    async fn test_search_book_mode_no_embeddings_returns_hint() {
        let (app, db) = test_app_with_embed_and_db(stub_embed(vec![1.0, 0.0]));
        // Seed a book with chapters but no chapter embeddings.
        let book = scylla_core::types::Book {
            title: "Book A".into(),
            url: "book-a".into(),
            status: scylla_core::types::BookStatus::Reading,
            sessions: vec![],
            active_session_id: None,
            tags: vec![],
            cover_url: None,
            description: None,
            chapters: vec![scylla_core::types::Chapter {
                url: "ch-a1".into(),
                title: "Chapter A1".into(),
                order: 1,
            }],
        };
        db.lock().await.upsert_book(&book).unwrap();

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/search")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"query":"x","mode":"book","limit":10}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp).await;
        assert_eq!(json["error"], "no_embeddings");
        assert_eq!(json["embedded"], 0);
        assert_eq!(json["total"], 1);
    }

    #[tokio::test]
    async fn test_search_book_mode_no_embeddings_beats_embedder_503() {
        // Offline first run: the embedder is in cooldown, but an empty corpus
        // must still produce the hint in book mode, not a generic 503.
        let (app, db) = test_app_with_embedder_in_cooldown_and_db();
        let book = scylla_core::types::Book {
            title: "Book A".into(),
            url: "book-a".into(),
            status: scylla_core::types::BookStatus::Reading,
            sessions: vec![],
            active_session_id: None,
            tags: vec![],
            cover_url: None,
            description: None,
            chapters: vec![scylla_core::types::Chapter {
                url: "ch-a1".into(),
                title: "Chapter A1".into(),
                order: 1,
            }],
        };
        db.lock().await.upsert_book(&book).unwrap();

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/search")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"query":"x","mode":"book","limit":10}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp).await;
        assert_eq!(json["error"], "no_embeddings");
        assert_eq!(json["embedded"], 0);
        assert_eq!(json["total"], 1);
    }

    /// Shared wire-contract fixture — MUST stay byte-identical to the fixture
    /// in `scylla-reader/src/storage/client.rs`
    /// (`test_parse_chapter_mode_wire_contract`). The server test asserts the
    /// route produces exactly this body; the TUI test parses the same literal.
    /// This is the canonical serde_json form (keys sorted alphabetically).
    ///
    /// NOTE: the server now always sends `snippet`; the TUI fixture may lag
    /// behind (its `#[serde(default)]` tolerates the missing field) until the
    /// designer syncs it.
    const CHAPTER_MODE_WIRE_FIXTURE: &str = r#"{"embedded":1,"hits":[{"chapter_idx":1,"score":100.0,"snippet":"chunk","title":"Chapter A1","url":"ch-a1"}],"mode":"chapter","total":1}"#;

    #[tokio::test]
    async fn test_search_chapter_mode_wire_contract() {
        let (app, db) = test_app_with_embed_and_db(stub_embed(vec![1.0, 0.0]));
        let book = scylla_core::types::Book {
            title: "Book A".into(),
            url: "book-a".into(),
            status: scylla_core::types::BookStatus::Reading,
            sessions: vec![],
            active_session_id: None,
            tags: vec![],
            cover_url: None,
            description: None,
            chapters: vec![scylla_core::types::Chapter {
                url: "ch-a1".into(),
                title: "Chapter A1".into(),
                order: 1,
            }],
        };
        db.lock().await.upsert_book(&book).unwrap();
        db.lock()
            .await
            .upsert_chapter_chunks("ch-a1", Some("book-a"), &[(0, "chunk", &[1.0, 0.0])], None)
            .unwrap();

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/search")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"query":"x","mode":"chapter","limit":10}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp).await;
        let expected: serde_json::Value = serde_json::from_str(CHAPTER_MODE_WIRE_FIXTURE).unwrap();
        assert_eq!(json, expected);
        // Byte-for-byte: the canonical serialization matches the shared fixture.
        assert_eq!(
            serde_json::to_string(&json).unwrap(),
            CHAPTER_MODE_WIRE_FIXTURE
        );
    }

    #[tokio::test]
    async fn test_embedding_status_route() {
        let (app, db) = test_app_with_embed_and_db(stub_embed(vec![1.0, 0.0]));
        let book = scylla_core::types::Book {
            title: "Book A".into(),
            url: "book-a".into(),
            status: scylla_core::types::BookStatus::Reading,
            sessions: vec![],
            active_session_id: None,
            tags: vec![],
            cover_url: None,
            description: None,
            chapters: vec![
                scylla_core::types::Chapter {
                    url: "ch-a1".into(),
                    title: "Chapter A1".into(),
                    order: 1,
                },
                scylla_core::types::Chapter {
                    url: "ch-a2".into(),
                    title: "Chapter A2".into(),
                    order: 2,
                },
            ],
        };
        db.lock().await.upsert_book(&book).unwrap();
        db.lock()
            .await
            .upsert_chapter_chunks("ch-a1", Some("book-a"), &[(0, "chunk", &[1.0, 0.0])], None)
            .unwrap();
        db.lock()
            .await
            .upsert_chapter_chunks("ch-a2", Some("book-a"), &[(0, "chunk", &[0.9, 0.1])], None)
            .unwrap();
        db.lock()
            .await
            .upsert_book_embedding(
                "book-a",
                None,
                Some(&[1.0, 0.0]),
                Some(&["Fantasy".to_string()]),
            )
            .unwrap();

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/books/book-a/embedding-status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp).await;
        assert_eq!(json["embedded_chapters"], 2);
        assert_eq!(json["total_chapters"], 2);
        assert_eq!(json["aggregate"], true);
        assert_eq!(json["genres"][0], "Fantasy");
        let urls = json["embedded_chapter_urls"].as_array().unwrap();
        assert_eq!(urls.len(), 2);
        assert_eq!(urls[0], "ch-a1");
        assert_eq!(urls[1], "ch-a2");
    }

    #[tokio::test]
    async fn test_embedding_status_missing_book_returns_404() {
        let app = test_app();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/books/nope/embedding-status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    /// Reproduces the embedding thread's Chapter handling: skip check → embed →
    /// find book → upsert. Verifies the embedding-status reflects the stored row.
    #[tokio::test]
    async fn test_embed_chapter_flow_stores_embedding() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let db = Arc::new(Mutex::new(db::ServerDb::open_conn(conn).unwrap()));
        let book = scylla_core::types::Book {
            title: "Book A".into(),
            url: "book-a".into(),
            status: scylla_core::types::BookStatus::Reading,
            sessions: vec![],
            active_session_id: None,
            tags: vec![],
            cover_url: None,
            description: None,
            chapters: vec![scylla_core::types::Chapter {
                url: "https://example.com/ch1".into(),
                title: "Chapter 1".into(),
                order: 1,
            }],
        };
        db.lock().await.upsert_book(&book).unwrap();

        let chapter_url = "https://example.com/ch1".to_string();
        let text = "chapter content".to_string();
        let hash = content_hash(&text);

        // Skip check: not embedded yet.
        let existing = db.lock().await.chapter_chunk_state(&chapter_url).unwrap();
        assert_eq!(existing.0, 0);

        // Embed (stub) + find book + upsert — mirroring the embedding thread.
        let embedding = vec![1.0, 0.0];
        let book_url = db
            .lock()
            .await
            .find_book_url_for_chapter(&chapter_url)
            .unwrap();
        assert_eq!(book_url.as_deref(), Some("book-a"));
        db.lock()
            .await
            .upsert_chapter_chunks(
                &chapter_url,
                book_url.as_deref(),
                &[(0, &text, &embedding)],
                Some(&hash),
            )
            .unwrap();

        // Embedding-status must now report 1 embedded chapter.
        let (embedded, total, _, _, urls) =
            db.lock().await.embedding_status("book-a").unwrap().unwrap();
        assert_eq!(embedded, 1);
        assert_eq!(total, 1);
        assert_eq!(urls, vec![chapter_url]);
    }

    #[test]
    fn test_should_skip_embedding() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let db = Arc::new(Mutex::new(db::ServerDb::open_conn(conn).unwrap()));
        let chapter_url = "https://example.com/ch1";
        let hash = content_hash("content");

        // Not embedded yet → don't skip.
        assert!(!should_skip_embedding(&db, chapter_url, &hash, 1));

        // Store one chunk with the same hash → skip.
        db.blocking_lock()
            .upsert_chapter_chunks(
                chapter_url,
                None,
                &[(0, "content", &[1.0, 0.0])],
                Some(&hash),
            )
            .unwrap();
        assert!(should_skip_embedding(&db, chapter_url, &hash, 1));

        // Different content hash → don't skip (content changed).
        let other_hash = content_hash("different content");
        assert!(!should_skip_embedding(&db, chapter_url, &other_hash, 1));

        // Same hash but a different chunk count → don't skip (partial embed).
        assert!(!should_skip_embedding(&db, chapter_url, &hash, 2));
    }

    #[test]
    fn test_process_embed_batch_stores_chapters() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let db = Arc::new(Mutex::new(db::ServerDb::open_conn(conn).unwrap()));
        let book = scylla_core::types::Book {
            title: "Book A".into(),
            url: "book-a".into(),
            status: scylla_core::types::BookStatus::Reading,
            sessions: vec![],
            active_session_id: None,
            tags: vec![],
            cover_url: None,
            description: None,
            chapters: vec![
                scylla_core::types::Chapter {
                    url: "https://example.com/ch1".into(),
                    title: "Ch1".into(),
                    order: 1,
                },
                scylla_core::types::Chapter {
                    url: "https://example.com/ch2".into(),
                    title: "Ch2".into(),
                    order: 2,
                },
            ],
        };
        db.blocking_lock().upsert_book(&book).unwrap();

        let embedder = embeddings::SharedEmbedder::with_embed(stub_embed(vec![1.0, 0.0]));
        let (event_tx, event_rx) = std::sync::mpsc::channel();
        let batch = vec![
            EmbedRequest::Chapter {
                chapter_url: "https://example.com/ch1".into(),
                text: "content one".into(),
                job_id: Some(1),
            },
            EmbedRequest::Chapter {
                chapter_url: "https://example.com/ch2".into(),
                text: "content two".into(),
                job_id: None,
            },
        ];
        let mut chapter_counts = std::collections::HashMap::new();
        let mut genre_embeddings = None;
        process_embed_batch(
            &batch,
            &db,
            &embedder,
            &event_tx,
            &mut chapter_counts,
            &mut genre_embeddings,
        );

        let (embedded, total, _, _, urls) = db
            .blocking_lock()
            .embedding_status("book-a")
            .unwrap()
            .unwrap();
        assert_eq!(embedded, 2);
        assert_eq!(total, 2);
        assert_eq!(urls.len(), 2);
        assert!(urls.contains(&"https://example.com/ch1".to_string()));
        assert!(urls.contains(&"https://example.com/ch2".to_string()));

        // The chapter with a job_id reports back a ChapterEmbedded event.
        let event = event_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap();
        assert!(matches!(
            event,
            scylla_core::messenger::AppEvent::ChapterEmbedded(1, url)
                if url == "https://example.com/ch1"
        ));
    }
}
