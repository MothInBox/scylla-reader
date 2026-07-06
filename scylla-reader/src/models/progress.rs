//! Progress tracking — current/total chapters, reading status.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Progress {
    pub current: u32,
    pub total: u32,
}
impl Progress {}
