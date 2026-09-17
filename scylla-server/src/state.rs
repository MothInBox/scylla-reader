use std::sync::Arc;
use tokio::sync::Mutex;

use crate::db::ServerDb;

pub struct AppState {
    pub db: Arc<Mutex<ServerDb>>,
    pub cmd_tx: std::sync::mpsc::Sender<scylla_core::messenger::AppCommand>,
    pub registry: Arc<std::sync::Mutex<scylla_core::scraper::ScraperRegistry>>,
    pub max_workers: std::sync::Mutex<u8>,
    pub rate_limit: std::sync::Mutex<u64>,
}
