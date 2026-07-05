use crate::models::Progress;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: i64,
    pub book_url: String,
    pub name: String,
    pub progress: Progress,
    pub created_at: String,
    pub updated_at: String,
}
