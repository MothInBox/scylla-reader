//! Domain models — re-exported from scylla_core.

pub use scylla_core::types::{Book, BookStatus, Chapter, Progress, Session};

/// Re-export so `crate::models::book::BookStatus` etc. still work.
pub mod book {
    pub use scylla_core::types::{Book, BookStatus, Chapter};
}
