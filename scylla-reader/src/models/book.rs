use serde::{Deserialize, Serialize};
use crate::models::Progress;

#[derive(PartialEq, Clone, Debug, Serialize, Deserialize)]
pub enum BookStatus {
    Reading,
    Paused,
    Dropped,
    Completed,
}

impl BookStatus {
    pub fn next(&self) -> BookStatus {
        match self {
            BookStatus::Reading   => BookStatus::Paused,
            BookStatus::Paused    => BookStatus::Dropped,
            BookStatus::Dropped   => BookStatus::Completed,
            BookStatus::Completed => BookStatus::Reading,
        }
    }
}

impl std::fmt::Display for BookStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            BookStatus::Reading   => write!(f, "Reading"),
            BookStatus::Paused    => write!(f, "Paused"),
            BookStatus::Dropped   => write!(f, "Dropped"),
            BookStatus::Completed => write!(f, "Completed"),
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Chapter {
    pub title: String,
    pub url: String,
    pub order: u32,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Book {
    pub title: String,
    pub url: String,
    pub status: BookStatus,
    pub progress: Progress,
    pub tags: Vec<String>,
    pub cover_url: Option<String>,
    pub description: Option<String>,
    pub chapters: Vec<Chapter>,
}
