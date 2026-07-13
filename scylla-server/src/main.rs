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
    let (event_tx, _event_rx) = std::sync::mpsc::channel();

    let registry = Arc::new(std::sync::Mutex::new(
        scylla_core::scraper::ScraperRegistry::new(),
    ));

    let worker_registry = scylla_core::scraper::ScraperRegistry::new();
    std::thread::spawn(move || {
        let manager = scylla_core::worker::JobManager::new(
            cmd_rx,
            event_tx,
            worker_registry,
            (8, 12),
            ratatui_image::picker::ProtocolType::Halfblocks,
            4,
            2,
        );
        manager.run();
    });

    let app_state = Arc::new(AppState {
        db,
        cmd_tx,
        registry,
    });

    let app = Router::new()
        .route("/api/health", axum::routing::get(routes::health::health))
        .route("/api/books", axum::routing::get(routes::books::list_books).post(routes::books::create_book))
        .route("/api/books/{url}", axum::routing::get(routes::books::get_book).delete(routes::books::delete_book))
        .route("/api/books/{url}/status", axum::routing::patch(routes::books::update_status))
        .route("/api/books/{url}/sessions", axum::routing::get(routes::sessions::list_sessions))
        .route("/api/books/{url}/active-session", axum::routing::patch(routes::sessions::set_active_session))
        .route("/api/sessions/{id}/progress", axum::routing::patch(routes::sessions::update_progress))
        .route("/api/books/{book_url}/chapters/{chapter_url}/content", axum::routing::get(routes::chapters::get_chapter_content))
        .route("/api/settings", axum::routing::get(routes::settings::get_settings).patch(routes::settings::update_settings))
        .layer(tower_http::cors::CorsLayer::permissive())
        .with_state(app_state);

    let addr = format!("127.0.0.1:{}", port);
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
