mod db;

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

    let _db = db::ServerDb::open_path(&db_path).expect("Failed to open database");

    let (_cmd_tx, cmd_rx) = std::sync::mpsc::channel();
    let (event_tx, _event_rx) = std::sync::mpsc::channel();

    let _worker_thread = std::thread::spawn(move || {
        let registry = scylla_core::scraper::ScraperRegistry::new();
        let manager = scylla_core::worker::JobManager::new(
            cmd_rx,
            event_tx,
            registry,
            (8, 12),
            ratatui_image::picker::ProtocolType::Halfblocks,
            4,
            2,
        );
        manager.run();
    });

    let app = axum::Router::new()
        .route("/api/health", axum::routing::get(|| async { "ok" }));

    let addr = format!("127.0.0.1:{}", port);
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
