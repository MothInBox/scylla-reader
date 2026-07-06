use std::time::Instant;

pub type JobId = u64;

#[derive(Debug, Clone, PartialEq)]
pub enum JobKind {
    Scrape(String),
    FetchChapter(String, usize),
    FetchCover(String),
}

impl JobKind {
    pub fn target(&self) -> &str {
        match self {
            JobKind::Scrape(url) | JobKind::FetchCover(url) => url,
            JobKind::FetchChapter(url, _) => url,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            JobKind::Scrape(_) => "Scrape",
            JobKind::FetchChapter(_, _) => "FetchCh",
            JobKind::FetchCover(_) => "Cover",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum JobStatus {
    Queued,
    Running,
    Completed,
    Failed(String),
    Cancelled,
}

// stub for future use: JobPriority
#[derive(Debug, Clone, PartialEq)]
pub enum JobPriority {
    Normal,
    High,
}

impl Default for JobPriority {
    fn default() -> Self {
        JobPriority::Normal
    }
}

#[derive(Debug, Clone)]
pub struct Job {
    pub id: JobId,
    pub kind: JobKind,
    pub status: JobStatus,
    pub target: String,              // display string (title or URL)
    pub priority: JobPriority,       // stub for future use
    // stub for future use: grouped jobs (UpdateAll -> multiple Scrape)
    // group_id: Option<u64>,
    pub created_at: Instant,
    pub started_at: Option<Instant>,
    pub completed_at: Option<Instant>,
    pub error: Option<String>,
}

impl Job {
    pub fn new(id: JobId, kind: JobKind, target: String, priority: JobPriority) -> Self {
        Self {
            id,
            kind,
            status: JobStatus::Queued,
            target,
            priority,
            created_at: Instant::now(),
            started_at: None,
            completed_at: None,
            error: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum JobFilter {
    All,
    Running,
    Completed,
    Failed,
}

impl Default for JobFilter {
    fn default() -> Self {
        JobFilter::All
    }
}

impl std::fmt::Display for JobFilter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JobFilter::All => write!(f, "All"),
            JobFilter::Running => write!(f, "Running"),
            JobFilter::Completed => write!(f, "Completed"),
            JobFilter::Failed => write!(f, "Failed"),
        }
    }
}
