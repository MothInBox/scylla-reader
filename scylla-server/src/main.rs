mod db;
mod embeddings;
mod routes;
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

/// Embedding function used by the background embedder.
type EmbedFn = dyn Fn(&[&str]) -> anyhow::Result<Vec<Vec<f32>>>;

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

    let worker_registry = scylla_core::scraper::ScraperRegistry::new();
    let max_workers: u8 = 4;
    let rate_limit: u64 = 2;
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
    embed: &EmbedFn,
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
                match embed(&[desc_text]) {
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
fn content_hash(text: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Whether a chapter should be skipped because it's already embedded with the
/// same content hash.
fn should_skip_embedding(
    db: &Arc<tokio::sync::Mutex<db::ServerDb>>,
    chapter_url: &str,
    hash: &str,
) -> bool {
    db.blocking_lock()
        .get_chapter_embedding(chapter_url)
        .ok()
        .flatten()
        .map(|(_, existing)| existing.as_deref() == Some(hash))
        .unwrap_or(false)
}

/// Embeds a collected batch of requests in one forward pass, then stores each
/// result (per-request book_url lookup + upsert).
fn process_embed_batch(
    batch: &[EmbedRequest],
    embed_db: &Arc<tokio::sync::Mutex<db::ServerDb>>,
    thread_embedder: &embeddings::SharedEmbedder,
    event_tx: &std::sync::mpsc::Sender<scylla_core::messenger::AppEvent>,
    chapter_counts: &mut std::collections::HashMap<String, u32>,
    genre_embeddings: &mut Option<Vec<(String, Vec<f32>)>>,
) {
    // Per-request skip checks (chapters already embedded with the same hash).
    let mut pending: Vec<(EmbedRequest, String)> = Vec::new();
    for req in batch {
        match req {
            EmbedRequest::Chapter {
                chapter_url, text, ..
            } => {
                let hash = content_hash(text);
                if should_skip_embedding(embed_db, chapter_url, &hash) {
                    eprintln!("EMBED: chapter {chapter_url} already embedded, skipping");
                    continue;
                }
                pending.push((req.clone(), hash));
            }
            EmbedRequest::Description { .. } => {
                pending.push((req.clone(), String::new()));
            }
        }
    }
    if pending.is_empty() {
        return;
    }
    let Some(embed) = thread_embedder.get() else {
        eprintln!(
            "EMBED: embedder unavailable (model load failed or in cooldown) — \
             skipping batch of {}",
            pending.len()
        );
        return;
    };
    let texts: Vec<&str> = pending
        .iter()
        .map(|(req, _)| match req {
            EmbedRequest::Chapter { text, .. } => text.as_str(),
            EmbedRequest::Description { text, .. } => text.as_str(),
        })
        .collect();
    let embeddings = match embed(&texts) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("EMBED: batch embedding failed ({} texts): {e}", texts.len());
            // Report the failure back for every pending chapter with a job id.
            for (req, _) in &pending {
                if let EmbedRequest::Chapter {
                    chapter_url,
                    job_id: Some(job_id),
                    ..
                } = req
                {
                    let _ = event_tx.send(scylla_core::messenger::AppEvent::ChapterEmbeddedFailed(
                        *job_id,
                        chapter_url.clone(),
                    ));
                }
            }
            return;
        }
    };
    for ((req, hash), embedding) in pending.iter().zip(embeddings.iter()) {
        match req {
            EmbedRequest::Chapter {
                chapter_url,
                job_id,
                ..
            } => {
                eprintln!("EMBED: embedding chapter {chapter_url}");
                let book_url = embed_db
                    .blocking_lock()
                    .find_book_url_for_chapter(chapter_url)
                    .ok()
                    .flatten();
                if book_url.is_none() {
                    eprintln!(
                        "EMBED: no book found for chapter {chapter_url} — \
                         storing without book association (embedding-status will not count it)"
                    );
                }
                let db = embed_db.blocking_lock();
                if let Err(e) = db.upsert_chapter_embedding(
                    chapter_url,
                    book_url.as_deref(),
                    embedding,
                    Some(hash),
                ) {
                    eprintln!("EMBED: failed to store chapter embedding: {e}");
                    if let Some(job_id) = job_id {
                        let _ =
                            event_tx.send(scylla_core::messenger::AppEvent::ChapterEmbeddedFailed(
                                *job_id,
                                chapter_url.clone(),
                            ));
                    }
                    continue;
                }
                drop(db);
                eprintln!(
                    "EMBED: stored embedding for chapter {chapter_url} (book: {:?})",
                    book_url
                );
                if let Some(job_id) = job_id {
                    let _ = event_tx.send(scylla_core::messenger::AppEvent::ChapterEmbedded(
                        *job_id,
                        chapter_url.clone(),
                    ));
                }
                if let Some(book_url) = book_url {
                    let count = chapter_counts.entry(book_url.clone()).or_insert(0);
                    *count += 1;
                    if should_recompute(*count) {
                        recompute_aggregate(embed_db, &book_url, &*embed, genre_embeddings);
                    }
                }
            }
            EmbedRequest::Description { book_url, .. } => {
                eprintln!("EMBED: embedding description for {book_url}");
                let db = embed_db.blocking_lock();
                if let Err(e) = db.upsert_book_embedding(book_url, Some(embedding), None, None) {
                    eprintln!("EMBED: failed to store description embedding: {e}");
                    continue;
                }
                drop(db);
                eprintln!("EMBED: stored description embedding for {book_url}");
                recompute_aggregate(embed_db, book_url, &*embed, genre_embeddings);
            }
        }
    }
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
                autoembed: Arc::new(std::sync::Mutex::new(false)),
            });
        build_router(state)
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

    fn stub_embed(vec: Vec<f32>) -> embeddings::EmbedFn {
        Arc::new(move |texts: &[&str]| Ok(vec![vec.clone(); texts.len()]))
    }

    #[tokio::test]
    async fn test_search_book_mode_ranks_by_similarity() {
        let (app, db) = test_app_with_embed_and_db(stub_embed(vec![1.0, 0.0]));
        // Seed book A (agg [1,0,0]) and book B (agg [0,1,0]).
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
        let results = json["results"].as_array().unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0]["book_url"], "book-a");
        assert_eq!(results[1]["book_url"], "book-b");
        assert!(results[0]["score"].as_f64().unwrap() > results[1]["score"].as_f64().unwrap());
        assert_eq!(results[0]["genres"][0], "Fantasy");
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
            .upsert_chapter_embedding("ch-a1", Some("book-a"), &[1.0, 0.0], None)
            .unwrap();
        db.lock()
            .await
            .upsert_chapter_embedding("ch-a2", Some("book-a"), &[0.9, 0.1], None)
            .unwrap();
        db.lock()
            .await
            .upsert_chapter_embedding("ch-b1", Some("book-b"), &[0.0, 1.0], None)
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
        let results = json["results"].as_array().unwrap();
        assert_eq!(results.len(), 3);
        // Flat, sorted by score desc.
        assert_eq!(results[0]["chapter_url"], "ch-a1");
        assert_eq!(results[0]["book_url"], "book-a");
        assert_eq!(results[0]["book_title"], "Book A");
        assert_eq!(results[0]["chapter_title"], "Chapter A1");
        assert_eq!(results[0]["chapter_idx"], 1);
        assert_eq!(results[0]["genres"][0], "Fantasy");
        assert_eq!(results[1]["chapter_url"], "ch-a2");
        assert_eq!(results[2]["chapter_url"], "ch-b1");
        assert_eq!(results[2]["book_title"], "Book B");
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
            assert_eq!(json["results"].as_array().unwrap().len(), expected);
        }
    }

    #[tokio::test]
    async fn test_search_returns_503_when_embedder_unavailable() {
        let app = test_app_with_embedder_in_cooldown();
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
            .upsert_chapter_embedding("ch-a1", Some("book-a"), &[1.0, 0.0], None)
            .unwrap();
        db.lock()
            .await
            .upsert_chapter_embedding("ch-a2", Some("book-a"), &[0.9, 0.1], None)
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
        let existing = db.lock().await.get_chapter_embedding(&chapter_url).unwrap();
        assert!(existing.is_none());

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
            .upsert_chapter_embedding(&chapter_url, book_url.as_deref(), &embedding, Some(&hash))
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
        assert!(!should_skip_embedding(&db, chapter_url, &hash));

        // Store an embedding with the same hash → skip.
        db.blocking_lock()
            .upsert_chapter_embedding(chapter_url, None, &[1.0, 0.0], Some(&hash))
            .unwrap();
        assert!(should_skip_embedding(&db, chapter_url, &hash));

        // Different content hash → don't skip (content changed).
        let other_hash = content_hash("different content");
        assert!(!should_skip_embedding(&db, chapter_url, &other_hash));
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
