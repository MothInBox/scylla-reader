//! Domain models — re-exported from scylla_core.

pub use scylla_core::types::{
    Book, BookStatus, Chapter, Job, JobFilter, JobId, JobKind, JobOutcome, JobPriority, JobStatus,
    Progress, Session,
};

/// Re-export so `crate::models::job::*` still works.
pub mod job {
    pub use scylla_core::types::{
        Job, JobFilter, JobId, JobKind, JobOutcome, JobPriority, JobStatus,
    };
}

/// Re-export so `crate::models::book::BookStatus` etc. still work.
pub mod book {
    pub use scylla_core::types::{Book, BookStatus, Chapter};
}
