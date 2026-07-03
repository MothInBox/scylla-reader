//! Domain models — Book, Chapter, Progress, BookStatus, LibraryFilter.

pub mod book;
pub mod progress;

pub use book::{Book, BookStatus, Chapter};
pub use progress::Progress;
