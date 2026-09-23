use std::sync::OnceLock;
use std::time::Instant;

use serde::{Deserialize, Serialize};

pub type JobId = u64;

/// A chapter reference for an `EmbedBatch` job.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChapterRef {
    pub url: String,
    pub idx: usize,
    pub title: String,
}

/// Per-chapter status within an `EmbedBatch` job's detail.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChapterDetail {
    pub title: String,
    pub url: String,
    /// "Pending" | "Done" | "Failed"
    pub status: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum JobKind {
    Scrape(String),
    FetchChapter(String, usize),
    FetchCover(String),
    EmbedBatch(Vec<ChapterRef>),
    InstallPlugin(String),
}

impl JobKind {
    pub fn target(&self) -> &str {
        match self {
            JobKind::Scrape(url) | JobKind::FetchCover(url) | JobKind::InstallPlugin(url) => url,
            JobKind::FetchChapter(url, _) => url,
            JobKind::EmbedBatch(chapters) => chapters.first().map(|c| c.url.as_str()).unwrap_or(""),
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            JobKind::Scrape(_) => "Scrape",
            JobKind::FetchChapter(_, _) => "FetchCh",
            JobKind::FetchCover(_) => "Cover",
            JobKind::EmbedBatch(_) => "EmbedBatch",
            JobKind::InstallPlugin(_) => "InstallPlugin",
        }
    }

    /// Stable wire-format name for the job kind (used by `JobDto` and SSE).
    pub fn as_str(&self) -> &'static str {
        match self {
            JobKind::Scrape(_) => "Scrape",
            JobKind::FetchChapter(_, _) => "FetchChapter",
            JobKind::FetchCover(_) => "FetchCover",
            JobKind::EmbedBatch(_) => "EmbedBatch",
            JobKind::InstallPlugin(_) => "InstallPlugin",
        }
    }

    /// The chapter refs for an `EmbedBatch` job, `None` for other kinds.
    pub fn chapters(&self) -> Option<&[ChapterRef]> {
        match self {
            JobKind::EmbedBatch(chapters) => Some(chapters),
            _ => None,
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

impl JobStatus {
    /// Stable wire-format name for the job status (used by `JobDto` and SSE).
    pub fn as_str(&self) -> &'static str {
        match self {
            JobStatus::Queued => "Queued",
            JobStatus::Running => "Running",
            JobStatus::Completed => "Completed",
            JobStatus::Failed(_) => "Failed",
            JobStatus::Cancelled => "Cancelled",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub enum JobPriority {
    #[default]
    Normal,
    High,
}

impl JobPriority {
    /// Stable wire-format name for the job priority (used by `JobDto`).
    pub fn as_str(&self) -> &'static str {
        match self {
            JobPriority::Normal => "Normal",
            JobPriority::High => "High",
        }
    }
}

#[derive(Debug, Clone)]
pub enum JobOutcome {
    BookScraped {
        title: String,
        chapters: usize,
        cover: bool,
    },
    ChapterFetched {
        title: String,
        content_chars: usize,
    },
    CoverFetched,
    PluginInstalled {
        /// All (domain, path) pairs installed by the job.
        plugins: Vec<(String, String)>,
    },
}

#[derive(Debug, Clone)]
pub struct Job {
    pub id: JobId,
    pub kind: JobKind,
    pub status: JobStatus,
    pub target: String,
    pub priority: JobPriority,
    pub created_at: Instant,
    pub started_at: Option<Instant>,
    pub completed_at: Option<Instant>,
    pub error: Option<String>,
    pub outcome: Option<JobOutcome>,
    /// Per-chapter detail for `EmbedBatch` jobs (e.g. "Pending"/"Done"/"Failed").
    pub detail: Option<Vec<ChapterDetail>>,
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
            outcome: None,
            detail: None,
        }
    }
}

/// Monotonic reference point for `JobDto` timestamps.
///
/// `std::time::Instant` is not serializable, so job timestamps are exported as
/// millis since this reference point (captured on first conversion, i.e. shortly
/// after process start). The values are monotonic and only meaningful relative to
/// each other — the TUI displays relative times, never absolute wall-clock.
fn process_start() -> Instant {
    static START: OnceLock<Instant> = OnceLock::new();
    *START.get_or_init(Instant::now)
}

/// Current time in millis since the monotonic reference point used by `JobDto`.
///
/// Used by the server to stamp `started_at_ms`/`completed_at_ms` when applying
/// `JobStatusChanged` events to the job snapshot.
pub fn job_now_ms() -> u64 {
    Instant::now().duration_since(process_start()).as_millis() as u64
}

/// Serializable snapshot of a job, safe to send over the wire (SSE / REST).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JobDto {
    pub id: u64,
    /// "Scrape" | "FetchChapter" | "FetchCover" | "EmbedBatch"
    pub kind: String,
    /// "Queued" | "Running" | "Completed" | "Failed" | "Cancelled"
    pub status: String,
    pub target: String,
    pub priority: String,
    /// Chapter index for `FetchChapter` jobs, `None` for other kinds.
    pub chapter_idx: Option<usize>,
    /// Millis since the monotonic reference point (see [`process_start`]).
    pub created_at_ms: u64,
    pub started_at_ms: Option<u64>,
    pub completed_at_ms: Option<u64>,
    pub error: Option<String>,
    pub outcome: Option<JobOutcomeDto>,
    /// Per-chapter detail for `EmbedBatch` jobs.
    pub detail: Option<Vec<ChapterDetail>>,
}

/// Serializable job outcome payload.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct JobOutcomeDto {
    /// "BookScraped" | "ChapterFetched" | "CoverFetched" | "PluginInstalled"
    pub kind: String,
    pub title: Option<String>,
    pub chapters: Option<usize>,
    pub cover: Option<bool>,
    pub content_chars: Option<usize>,
    /// All (domain, path) pairs for a `PluginInstalled` outcome.
    pub plugins: Option<Vec<(String, String)>>,
}

impl From<&JobOutcome> for JobOutcomeDto {
    fn from(outcome: &JobOutcome) -> Self {
        match outcome {
            JobOutcome::BookScraped {
                title,
                chapters,
                cover,
            } => Self {
                kind: "BookScraped".to_string(),
                title: Some(title.clone()),
                chapters: Some(*chapters),
                cover: Some(*cover),
                ..Self::default()
            },
            JobOutcome::ChapterFetched {
                title,
                content_chars,
            } => Self {
                kind: "ChapterFetched".to_string(),
                title: Some(title.clone()),
                content_chars: Some(*content_chars),
                ..Self::default()
            },
            JobOutcome::CoverFetched => Self {
                kind: "CoverFetched".to_string(),
                ..Self::default()
            },
            JobOutcome::PluginInstalled { plugins } => Self {
                kind: "PluginInstalled".to_string(),
                plugins: Some(plugins.clone()),
                ..Self::default()
            },
        }
    }
}

impl From<&Job> for JobDto {
    fn from(job: &Job) -> Self {
        let start = process_start();
        let to_ms = |t: Instant| t.duration_since(start).as_millis() as u64;
        Self {
            id: job.id,
            kind: job.kind.as_str().to_string(),
            status: job.status.as_str().to_string(),
            target: job.target.clone(),
            priority: job.priority.as_str().to_string(),
            chapter_idx: match &job.kind {
                JobKind::FetchChapter(_, idx) => Some(*idx),
                _ => None,
            },
            created_at_ms: to_ms(job.created_at),
            started_at_ms: job.started_at.map(to_ms),
            completed_at_ms: job.completed_at.map(to_ms),
            error: job.error.clone(),
            outcome: job.outcome.as_ref().map(JobOutcomeDto::from),
            detail: job.detail.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub enum JobFilter {
    #[default]
    All,
    Running,
    Completed,
    Failed,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_job(id: JobId, kind: JobKind) -> Job {
        let target = kind.target().to_string();
        Job::new(id, kind, target, JobPriority::Normal)
    }

    #[test]
    fn test_job_dto_from_queued_job() {
        let job = sample_job(1, JobKind::Scrape("http://example.com".into()));
        let dto = JobDto::from(&job);
        assert_eq!(dto.id, 1);
        assert_eq!(dto.kind, "Scrape");
        assert_eq!(dto.status, "Queued");
        assert_eq!(dto.target, "http://example.com");
        assert_eq!(dto.priority, "Normal");
        assert_eq!(dto.chapter_idx, None);
        assert!(dto.started_at_ms.is_none());
        assert!(dto.completed_at_ms.is_none());
        assert!(dto.error.is_none());
        assert!(dto.outcome.is_none());
    }

    #[test]
    fn test_job_dto_from_running_job() {
        let mut job = sample_job(2, JobKind::FetchChapter("http://example.com/ch1".into(), 3));
        job.status = JobStatus::Running;
        job.started_at = Some(Instant::now());
        let dto = JobDto::from(&job);
        assert_eq!(dto.kind, "FetchChapter");
        assert_eq!(dto.status, "Running");
        assert_eq!(dto.chapter_idx, Some(3));
        assert!(dto.started_at_ms.is_some());
        assert!(dto.completed_at_ms.is_none());
    }

    #[test]
    fn test_job_dto_from_failed_job() {
        let mut job = sample_job(3, JobKind::FetchCover("http://example.com/c.jpg".into()));
        job.status = JobStatus::Failed("network error".into());
        job.completed_at = Some(Instant::now());
        job.error = Some("network error".into());
        let dto = JobDto::from(&job);
        assert_eq!(dto.kind, "FetchCover");
        assert_eq!(dto.status, "Failed");
        assert_eq!(dto.chapter_idx, None);
        assert_eq!(dto.error.as_deref(), Some("network error"));
        assert!(dto.completed_at_ms.is_some());
    }

    #[test]
    fn test_job_dto_from_cancelled_job() {
        let mut job = sample_job(4, JobKind::Scrape("http://example.com".into()));
        job.status = JobStatus::Cancelled;
        let dto = JobDto::from(&job);
        assert_eq!(dto.status, "Cancelled");
    }

    #[test]
    fn test_job_dto_with_book_scraped_outcome() {
        let mut job = sample_job(5, JobKind::Scrape("http://example.com".into()));
        job.status = JobStatus::Completed;
        job.outcome = Some(JobOutcome::BookScraped {
            title: "Book".into(),
            chapters: 10,
            cover: true,
        });
        let dto = JobDto::from(&job);
        let outcome = dto.outcome.expect("outcome should be set");
        assert_eq!(outcome.kind, "BookScraped");
        assert_eq!(outcome.title.as_deref(), Some("Book"));
        assert_eq!(outcome.chapters, Some(10));
        assert_eq!(outcome.cover, Some(true));
        assert_eq!(outcome.content_chars, None);
    }

    #[test]
    fn test_job_dto_with_chapter_fetched_outcome() {
        let mut job = sample_job(6, JobKind::FetchChapter("http://example.com/ch1".into(), 0));
        job.status = JobStatus::Completed;
        job.outcome = Some(JobOutcome::ChapterFetched {
            title: "Ch1".into(),
            content_chars: 500,
        });
        let dto = JobDto::from(&job);
        let outcome = dto.outcome.expect("outcome should be set");
        assert_eq!(outcome.kind, "ChapterFetched");
        assert_eq!(outcome.title.as_deref(), Some("Ch1"));
        assert_eq!(outcome.content_chars, Some(500));
        assert_eq!(outcome.chapters, None);
    }

    #[test]
    fn test_job_dto_with_cover_fetched_outcome() {
        let mut job = sample_job(7, JobKind::FetchCover("http://example.com/c.jpg".into()));
        job.status = JobStatus::Completed;
        job.outcome = Some(JobOutcome::CoverFetched);
        let dto = JobDto::from(&job);
        let outcome = dto.outcome.expect("outcome should be set");
        assert_eq!(outcome.kind, "CoverFetched");
        assert!(outcome.title.is_none());
        assert!(outcome.chapters.is_none());
        assert!(outcome.cover.is_none());
        assert!(outcome.content_chars.is_none());
    }

    #[test]
    fn test_job_dto_timestamps_are_monotonic() {
        let job = sample_job(8, JobKind::Scrape("http://example.com".into()));
        let dto = JobDto::from(&job);
        // created_at_ms must be a small non-negative relative value.
        assert!(dto.created_at_ms < 60_000);
        let now = job_now_ms();
        assert!(now >= dto.created_at_ms);
    }

    #[test]
    fn test_job_dto_serde_roundtrip() {
        let mut job = sample_job(9, JobKind::FetchChapter("http://example.com/ch1".into(), 4));
        job.status = JobStatus::Completed;
        job.outcome = Some(JobOutcome::ChapterFetched {
            title: "Ch1".into(),
            content_chars: 300,
        });
        let dto = JobDto::from(&job);
        assert_eq!(dto.chapter_idx, Some(4));
        let json = serde_json::to_string(&dto).unwrap();
        let back: JobDto = serde_json::from_str(&json).unwrap();
        assert_eq!(dto, back);
    }

    #[test]
    fn test_job_kind_status_priority_as_str() {
        assert_eq!(JobKind::Scrape("u".into()).as_str(), "Scrape");
        assert_eq!(
            JobKind::FetchChapter("u".into(), 0).as_str(),
            "FetchChapter"
        );
        assert_eq!(JobKind::FetchCover("u".into()).as_str(), "FetchCover");
        assert_eq!(JobKind::EmbedBatch(vec![]).as_str(), "EmbedBatch");
        assert_eq!(JobKind::EmbedBatch(vec![]).label(), "EmbedBatch");
        assert_eq!(
            JobKind::InstallPlugin("https://github.com/o/r".into()).as_str(),
            "InstallPlugin"
        );
        assert_eq!(
            JobKind::InstallPlugin("https://github.com/o/r".into()).target(),
            "https://github.com/o/r"
        );
        assert_eq!(JobStatus::Queued.as_str(), "Queued");
        assert_eq!(JobStatus::Running.as_str(), "Running");
        assert_eq!(JobStatus::Completed.as_str(), "Completed");
        assert_eq!(JobStatus::Failed("e".into()).as_str(), "Failed");
        assert_eq!(JobStatus::Cancelled.as_str(), "Cancelled");
        assert_eq!(JobPriority::Normal.as_str(), "Normal");
        assert_eq!(JobPriority::High.as_str(), "High");
    }

    #[test]
    fn test_chapter_ref_serde_roundtrip() {
        let ch = ChapterRef {
            url: "http://example.com/ch1".into(),
            idx: 3,
            title: "Chapter 3".into(),
        };
        let json = serde_json::to_string(&ch).unwrap();
        let back: ChapterRef = serde_json::from_str(&json).unwrap();
        assert_eq!(ch, back);
    }

    #[test]
    fn test_chapter_detail_serde_roundtrip() {
        let d = ChapterDetail {
            title: "Chapter 3".into(),
            url: "http://example.com/ch1".into(),
            status: "Done".into(),
        };
        let json = serde_json::to_string(&d).unwrap();
        let back: ChapterDetail = serde_json::from_str(&json).unwrap();
        assert_eq!(d, back);
    }

    #[test]
    fn test_job_dto_detail_conversion() {
        let chapters = vec![ChapterRef {
            url: "http://example.com/ch1".into(),
            idx: 0,
            title: "Ch1".into(),
        }];
        let mut job = Job::new(
            10,
            JobKind::EmbedBatch(chapters),
            "Embed 1 chapters".into(),
            JobPriority::High,
        );
        job.detail = Some(vec![ChapterDetail {
            title: "Ch1".into(),
            url: "http://example.com/ch1".into(),
            status: "Pending".into(),
        }]);
        let dto = JobDto::from(&job);
        assert_eq!(dto.kind, "EmbedBatch");
        let detail = dto.detail.unwrap();
        assert_eq!(detail.len(), 1);
        assert_eq!(detail[0].status, "Pending");
        assert_eq!(detail[0].title, "Ch1");
    }

    #[test]
    fn test_embed_batch_target_is_first_chapter_url() {
        let chapters = vec![
            ChapterRef {
                url: "http://example.com/ch1".into(),
                idx: 0,
                title: "Ch1".into(),
            },
            ChapterRef {
                url: "http://example.com/ch2".into(),
                idx: 1,
                title: "Ch2".into(),
            },
        ];
        let kind = JobKind::EmbedBatch(chapters);
        assert_eq!(kind.target(), "http://example.com/ch1");
        assert_eq!(JobKind::EmbedBatch(vec![]).target(), "");
    }
}
