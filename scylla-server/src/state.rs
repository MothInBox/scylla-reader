use std::sync::Arc;
use tokio::sync::Mutex;

use crate::db::ServerDb;

pub struct AppState {
    pub db: Arc<Mutex<ServerDb>>,
    pub cmd_tx: std::sync::mpsc::Sender<scylla_core::messenger::AppCommand>,
    pub registry: Arc<std::sync::Mutex<scylla_core::scraper::ScraperRegistry>>,
    pub max_workers: std::sync::Mutex<u8>,
    pub rate_limit: std::sync::Mutex<u64>,
    /// Current job snapshot, maintained by the event consumer thread.
    pub jobs: Arc<std::sync::Mutex<Vec<scylla_core::types::JobDto>>>,
    /// Broadcast channel for job/app events, streamed to SSE clients.
    ///
    /// `AppEvent` is not `Clone` (its `CoverFetched` payload is a non-cloneable
    /// `StatefulProtocol`), so events are wrapped in `Arc` for broadcasting.
    pub job_events: tokio::sync::broadcast::Sender<Arc<scylla_core::messenger::AppEvent>>,
    /// Shared, lazily-loaded embedder (used by the embedding thread and routes).
    ///
    /// Read by Phase 4's search endpoint; the embedding thread holds its own
    /// clone of the same `Arc`.
    #[allow(dead_code)]
    pub embedder: Arc<crate::embeddings::SharedEmbedder>,
    /// Shared, lazily-loaded cross-encoder reranker (search only).
    pub reranker: Arc<crate::embeddings::SharedCrossEncoder>,
    /// Whether scraped chapters/descriptions are embedded automatically.
    pub autoembed: Arc<std::sync::Mutex<bool>>,
}
