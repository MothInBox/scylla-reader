mod db;
mod routes;
mod state;

use std::sync::Arc;

use axum::Router;
use state::AppState;
use tokio::sync::Mutex;

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
            (8, 12),
            ratatui_image::picker::ProtocolType::Halfblocks,
            max_workers,
            rate_limit,
        );
        manager.run();
    });

    let event_db = db.clone();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create event runtime");
        for event in event_rx {
            if let scylla_core::messenger::AppEvent::BookScraped(book) = event {
                let db = event_db.clone();
                let result = rt.block_on(async move {
                    let db = db.lock().await;
                    db.upsert_book(&book)
                });
                if let Err(e) = result {
                    eprintln!("Failed to persist scraped book: {e}");
                }
            }
        }
    });

    let app_state = Arc::new(AppState {
        db,
        cmd_tx,
        registry,
        max_workers: std::sync::Mutex::new(max_workers),
        rate_limit: std::sync::Mutex::new(rate_limit),
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
        .layer(tower_http::cors::CorsLayer::permissive())
        .with_state(app_state)
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
        let state = Arc::new(AppState {
            db,
            cmd_tx,
            registry,
            max_workers: std::sync::Mutex::new(4),
            rate_limit: std::sync::Mutex::new(2),
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
                    .body(Body::from(r#"{"max_workers":8,"rate_limit":5}"#))
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
    }
}
