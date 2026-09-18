//! Server event types — deserialized from the SSE stream and the jobs snapshot.
//!
//! The server owns all jobs and pushes events over `GET /api/jobs/stream`. Each
//! SSE event's data field is a JSON envelope internally tagged on `"type"`.
//! `JobsSnapshot`, `ConnectionState`, and `AiSearchResults` are synthesized by
//! the TUI (SSE client thread / search thread) and never appear on the wire.

use base64::Engine;
use scylla_core::types::{Book, ChapterDetail, JobDto, JobOutcomeDto};
use serde::Deserialize;
use std::sync::{Mutex, OnceLock, mpsc};

/// A single chapter hit from `POST /api/search` (chapter mode).
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ChapterHit {
    pub book_url: String,
    pub book_title: String,
    pub chapter_url: String,
    pub chapter_idx: usize,
    pub chapter_title: String,
    pub score: f32,
    #[serde(default)]
    pub genres: Vec<String>,
}

/// Chapters of one book, grouped for the AI results modal.
#[derive(Debug, Clone, PartialEq)]
pub struct ChapterGroup {
    pub book_url: String,
    pub book_title: String,
    pub genres: Vec<String>,
    /// Chapters sorted by score descending.
    pub chapters: Vec<ChapterHit>,
    /// Best (highest) chapter score — the book's ranking in the results.
    pub best_score: f32,
}

/// TUI-side event channel. `App::new` installs the sender; the one-shot AI
/// search thread uses it to deliver `AiSearchResults`.
static EVENT_TX: OnceLock<Mutex<Option<mpsc::Sender<ServerEvent>>>> = OnceLock::new();

fn event_tx_slot() -> &'static Mutex<Option<mpsc::Sender<ServerEvent>>> {
    EVENT_TX.get_or_init(|| Mutex::new(None))
}

/// Install the TUI's event sender (called once from `App::new`).
pub fn set_event_tx(tx: mpsc::Sender<ServerEvent>) {
    *event_tx_slot().lock().unwrap() = Some(tx);
}

/// Clone of the TUI's event sender, if installed.
pub fn event_tx() -> Option<mpsc::Sender<ServerEvent>> {
    event_tx_slot().lock().unwrap().clone()
}

#[derive(Clone)]
pub enum ServerEvent {
    JobEnqueued {
        job: JobDto,
    },
    JobStatusChanged {
        id: u64,
        status: String,
        error: Option<String>,
        started_at_ms: Option<u64>,
        completed_at_ms: Option<u64>,
    },
    JobOutcome {
        id: u64,
        outcome: JobOutcomeDto,
    },
    WorkersChanged {
        max_workers: u8,
    },
    BookScraped {
        book: Book,
    },
    ChapterFetched {
        url: String,
        chapter_idx: usize,
        title: String,
        content: String,
    },
    ChapterFetchFailed,
    /// The wire carries base64-encoded image bytes; decode them on deserialize.
    CoverFetched {
        url: String,
        bytes: Vec<u8>,
    },
    PluginInstalled {
        domain: String,
        path: String,
    },
    PluginInstallFailed {
        message: String,
    },
    /// TUI-internal: full job snapshot from `GET /api/jobs`.
    JobsSnapshot {
        jobs: Vec<JobDto>,
        server_now_ms: u64,
    },
    /// TUI-internal: SSE connection state. `error` is Some when disconnected
    /// (the reason the connection failed/lost), None when connected.
    ConnectionState {
        connected: bool,
        error: Option<String>,
    },
    /// TUI-internal: AI search results delivered by the one-shot search thread.
    AiSearchResults {
        query: String,
        result: Result<Vec<ChapterHit>, String>,
    },
    /// Per-chapter detail for an `EmbedBatch` job.
    JobDetailChanged {
        id: u64,
        detail: Vec<ChapterDetail>,
    },
    /// Server-internal: a chapter queued for embedding. The TUI ignores it.
    ChapterToEmbed {
        id: u64,
        chapter: serde_json::Value,
    },
    /// The embedding thread reports a chapter's embedding was stored.
    ChapterEmbedded {
        id: u64,
        url: String,
    },
    /// The embedding thread reports a chapter whose embedding FAILED.
    ChapterEmbeddedFailed {
        id: u64,
        url: String,
    },
}

impl<'de> Deserialize<'de> for ServerEvent {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        // Raw wire shapes — `ChapterFetched` nests its fields under `chapter`
        // and `CoverFetched` carries base64, so they need a decode step.
        #[derive(Deserialize)]
        #[serde(tag = "type")]
        enum Wire {
            JobEnqueued {
                job: JobDto,
            },
            JobStatusChanged {
                id: u64,
                status: String,
                error: Option<String>,
                #[serde(default)]
                started_at_ms: Option<u64>,
                #[serde(default)]
                completed_at_ms: Option<u64>,
            },
            JobOutcome {
                id: u64,
                outcome: JobOutcomeDto,
            },
            WorkersChanged {
                max_workers: u8,
            },
            BookScraped {
                book: Book,
            },
            ChapterFetched {
                chapter: ChapterWire,
            },
            ChapterFetchFailed,
            CoverFetched {
                url: String,
                bytes: String,
            },
            PluginInstalled {
                domain: String,
                path: String,
            },
            PluginInstallFailed {
                message: String,
            },
            JobsSnapshot {
                jobs: Vec<JobDto>,
                server_now_ms: u64,
            },
            ConnectionState {
                connected: bool,
                #[serde(default)]
                error: Option<String>,
            },
            JobDetailChanged {
                id: u64,
                detail: Vec<ChapterDetail>,
            },
            ChapterToEmbed {
                id: u64,
                chapter: serde_json::Value,
            },
            ChapterEmbedded {
                id: u64,
                url: String,
            },
            ChapterEmbeddedFailed {
                id: u64,
                url: String,
            },
        }

        #[derive(Deserialize)]
        struct ChapterWire {
            #[serde(default)]
            url: String,
            chapter_idx: usize,
            title: String,
            content: String,
        }

        let wire = Wire::deserialize(deserializer)?;
        Ok(match wire {
            Wire::JobEnqueued { job } => ServerEvent::JobEnqueued { job },
            Wire::JobStatusChanged {
                id,
                status,
                error,
                started_at_ms,
                completed_at_ms,
            } => ServerEvent::JobStatusChanged {
                id,
                status,
                error,
                started_at_ms,
                completed_at_ms,
            },
            Wire::JobOutcome { id, outcome } => ServerEvent::JobOutcome { id, outcome },
            Wire::WorkersChanged { max_workers } => ServerEvent::WorkersChanged { max_workers },
            Wire::BookScraped { book } => ServerEvent::BookScraped { book },
            Wire::ChapterFetched { chapter } => ServerEvent::ChapterFetched {
                url: chapter.url,
                chapter_idx: chapter.chapter_idx,
                title: chapter.title,
                content: chapter.content,
            },
            Wire::ChapterFetchFailed => ServerEvent::ChapterFetchFailed,
            Wire::CoverFetched { url, bytes } => {
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(bytes.as_bytes())
                    .map_err(serde::de::Error::custom)?;
                ServerEvent::CoverFetched { url, bytes }
            }
            Wire::PluginInstalled { domain, path } => ServerEvent::PluginInstalled { domain, path },
            Wire::PluginInstallFailed { message } => ServerEvent::PluginInstallFailed { message },
            Wire::JobsSnapshot {
                jobs,
                server_now_ms,
            } => ServerEvent::JobsSnapshot {
                jobs,
                server_now_ms,
            },
            Wire::ConnectionState { connected, error } => {
                ServerEvent::ConnectionState { connected, error }
            }
            Wire::JobDetailChanged { id, detail } => ServerEvent::JobDetailChanged { id, detail },
            Wire::ChapterToEmbed { id, chapter } => ServerEvent::ChapterToEmbed { id, chapter },
            Wire::ChapterEmbedded { id, url } => ServerEvent::ChapterEmbedded { id, url },
            Wire::ChapterEmbeddedFailed { id, url } => {
                ServerEvent::ChapterEmbeddedFailed { id, url }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(json: &str) -> ServerEvent {
        serde_json::from_str(json).unwrap()
    }

    fn sample_job_json(id: u64) -> String {
        format!(
            r#"{{"id":{},"kind":"Scrape","status":"Queued","target":"http://example.com","priority":"Normal","chapter_idx":null,"created_at_ms":0,"started_at_ms":null,"completed_at_ms":null,"error":null,"outcome":null}}"#,
            id
        )
    }

    #[test]
    fn test_job_enqueued() {
        let event = parse(&format!(
            r#"{{"type":"JobEnqueued","job":{}}}"#,
            sample_job_json(1)
        ));
        match event {
            ServerEvent::JobEnqueued { job } => {
                assert_eq!(job.id, 1);
                assert_eq!(job.kind, "Scrape");
                assert_eq!(job.status, "Queued");
            }
            _ => panic!("expected JobEnqueued"),
        }
    }

    #[test]
    fn test_job_status_changed() {
        let event = parse(r#"{"type":"JobStatusChanged","id":2,"status":"Running","error":null}"#);
        match event {
            ServerEvent::JobStatusChanged {
                id,
                status,
                error,
                started_at_ms,
                completed_at_ms,
            } => {
                assert_eq!(id, 2);
                assert_eq!(status, "Running");
                assert!(error.is_none());
                // Backward compat: absent stamps default to None.
                assert!(started_at_ms.is_none());
                assert!(completed_at_ms.is_none());
            }
            _ => panic!("expected JobStatusChanged"),
        }
    }

    #[test]
    fn test_job_status_changed_failed_includes_error() {
        let event = parse(r#"{"type":"JobStatusChanged","id":2,"status":"Failed","error":"boom"}"#);
        match event {
            ServerEvent::JobStatusChanged {
                id, status, error, ..
            } => {
                assert_eq!(id, 2);
                assert_eq!(status, "Failed");
                assert_eq!(error.as_deref(), Some("boom"));
            }
            _ => panic!("expected JobStatusChanged"),
        }
    }

    #[test]
    fn test_job_status_changed_includes_server_stamps() {
        let event = parse(
            r#"{"type":"JobStatusChanged","id":2,"status":"Running","error":null,"started_at_ms":200,"completed_at_ms":null}"#,
        );
        match event {
            ServerEvent::JobStatusChanged {
                id,
                status,
                started_at_ms,
                completed_at_ms,
                ..
            } => {
                assert_eq!(id, 2);
                assert_eq!(status, "Running");
                assert_eq!(started_at_ms, Some(200));
                assert!(completed_at_ms.is_none());
            }
            _ => panic!("expected JobStatusChanged"),
        }
    }

    #[test]
    fn test_job_outcome() {
        let event = parse(
            r#"{"type":"JobOutcome","id":3,"outcome":{"kind":"BookScraped","title":"Book","chapters":5,"cover":true,"content_chars":null}}"#,
        );
        match event {
            ServerEvent::JobOutcome { id, outcome } => {
                assert_eq!(id, 3);
                assert_eq!(outcome.kind, "BookScraped");
                assert_eq!(outcome.title.as_deref(), Some("Book"));
                assert_eq!(outcome.chapters, Some(5));
                assert_eq!(outcome.cover, Some(true));
            }
            _ => panic!("expected JobOutcome"),
        }
    }

    #[test]
    fn test_workers_changed() {
        let event = parse(r#"{"type":"WorkersChanged","max_workers":6}"#);
        match event {
            ServerEvent::WorkersChanged { max_workers } => assert_eq!(max_workers, 6),
            _ => panic!("expected WorkersChanged"),
        }
    }

    #[test]
    fn test_book_scraped() {
        let event = parse(
            r#"{"type":"BookScraped","book":{"title":"Book","url":"http://example.com","status":"Reading","sessions":[],"active_session_id":null,"tags":[],"cover_url":null,"description":null,"chapters":[]}}"#,
        );
        match event {
            ServerEvent::BookScraped { book } => {
                assert_eq!(book.title, "Book");
                assert_eq!(book.url, "http://example.com");
            }
            _ => panic!("expected BookScraped"),
        }
    }

    #[test]
    fn test_chapter_fetched() {
        let event = parse(
            r#"{"type":"ChapterFetched","chapter":{"url":"http://example.com/ch1","chapter_idx":0,"title":"Ch1","content":"text"}}"#,
        );
        match event {
            ServerEvent::ChapterFetched {
                url,
                chapter_idx,
                title,
                content,
            } => {
                assert_eq!(url, "http://example.com/ch1");
                assert_eq!(chapter_idx, 0);
                assert_eq!(title, "Ch1");
                assert_eq!(content, "text");
            }
            _ => panic!("expected ChapterFetched"),
        }
    }

    #[test]
    fn test_chapter_fetched_url_defaults_when_missing() {
        // Older servers omit `url`; it must default to empty.
        let event = parse(
            r#"{"type":"ChapterFetched","chapter":{"chapter_idx":0,"title":"Ch1","content":"text"}}"#,
        );
        match event {
            ServerEvent::ChapterFetched { url, .. } => assert_eq!(url, ""),
            _ => panic!("expected ChapterFetched"),
        }
    }

    #[test]
    fn test_chapter_fetch_failed() {
        let event = parse(r#"{"type":"ChapterFetchFailed"}"#);
        assert!(matches!(event, ServerEvent::ChapterFetchFailed));
    }

    #[test]
    fn test_cover_fetched_base64() {
        let event =
            parse(r#"{"type":"CoverFetched","url":"http://example.com/c.jpg","bytes":"AQID"}"#);
        match event {
            ServerEvent::CoverFetched { url, bytes } => {
                assert_eq!(url, "http://example.com/c.jpg");
                assert_eq!(bytes, vec![1, 2, 3]);
            }
            _ => panic!("expected CoverFetched"),
        }
    }

    #[test]
    fn test_plugin_installed() {
        let event = parse(r#"{"type":"PluginInstalled","domain":"example.com","path":"/p.wasm"}"#);
        match event {
            ServerEvent::PluginInstalled { domain, path } => {
                assert_eq!(domain, "example.com");
                assert_eq!(path, "/p.wasm");
            }
            _ => panic!("expected PluginInstalled"),
        }
    }

    #[test]
    fn test_plugin_install_failed() {
        let event = parse(r#"{"type":"PluginInstallFailed","message":"boom"}"#);
        match event {
            ServerEvent::PluginInstallFailed { message } => assert_eq!(message, "boom"),
            _ => panic!("expected PluginInstallFailed"),
        }
    }

    #[test]
    fn test_jobs_snapshot() {
        let event = parse(&format!(
            r#"{{"type":"JobsSnapshot","jobs":[{},{}],"server_now_ms":12345}}"#,
            sample_job_json(1),
            sample_job_json(2)
        ));
        match event {
            ServerEvent::JobsSnapshot {
                jobs,
                server_now_ms,
            } => {
                assert_eq!(jobs.len(), 2);
                assert_eq!(jobs[0].id, 1);
                assert_eq!(jobs[1].id, 2);
                assert_eq!(server_now_ms, 12345);
            }
            _ => panic!("expected JobsSnapshot"),
        }
    }

    #[test]
    fn test_connection_state() {
        let event = parse(r#"{"type":"ConnectionState","connected":true}"#);
        match event {
            ServerEvent::ConnectionState { connected, error } => {
                assert!(connected);
                assert!(error.is_none());
            }
            _ => panic!("expected ConnectionState"),
        }
    }

    #[test]
    fn test_connection_state_with_error() {
        let event = parse(
            r#"{"type":"ConnectionState","connected":false,"error":"Failed to connect to http://mock: boom"}"#,
        );
        match event {
            ServerEvent::ConnectionState { connected, error } => {
                assert!(!connected);
                assert_eq!(
                    error.as_deref(),
                    Some("Failed to connect to http://mock: boom")
                );
            }
            _ => panic!("expected ConnectionState"),
        }
    }

    #[test]
    fn test_chapter_hit_deserializes_from_response_json() {
        let json = r#"{"book_url":"http://example.com/book","book_title":"Book","chapter_url":"http://example.com/book/ch1","chapter_idx":3,"chapter_title":"Ch3","score":0.87,"genres":["Fantasy","LitRPG"]}"#;
        let hit: ChapterHit = serde_json::from_str(json).unwrap();
        assert_eq!(hit.book_url, "http://example.com/book");
        assert_eq!(hit.book_title, "Book");
        assert_eq!(hit.chapter_url, "http://example.com/book/ch1");
        assert_eq!(hit.chapter_idx, 3);
        assert_eq!(hit.chapter_title, "Ch3");
        assert!((hit.score - 0.87).abs() < 1e-6);
        assert_eq!(hit.genres, vec!["Fantasy", "LitRPG"]);
    }

    #[test]
    fn test_chapter_hit_genres_default_when_missing() {
        let json = r#"{"book_url":"u","book_title":"B","chapter_url":"c","chapter_idx":0,"chapter_title":"C","score":0.5}"#;
        let hit: ChapterHit = serde_json::from_str(json).unwrap();
        assert!(hit.genres.is_empty());
    }

    #[test]
    fn test_ai_search_results_event_constructed_directly() {
        let event = ServerEvent::AiSearchResults {
            query: "dragon".into(),
            result: Ok(vec![]),
        };
        match event {
            ServerEvent::AiSearchResults { query, result } => {
                assert_eq!(query, "dragon");
                assert!(result.is_ok());
            }
            _ => panic!("expected AiSearchResults"),
        }
    }

    #[test]
    fn test_job_detail_changed_deserializes() {
        let event = parse(
            r#"{"type":"JobDetailChanged","id":7,"detail":[{"title":"1.1 Crappy Monday","url":"u1","status":"Done"},{"title":"2.1 New Semester","url":"u2","status":"Pending"}]}"#,
        );
        match event {
            ServerEvent::JobDetailChanged { id, detail } => {
                assert_eq!(id, 7);
                assert_eq!(detail.len(), 2);
                assert_eq!(detail[0].title, "1.1 Crappy Monday");
                assert_eq!(detail[0].status, "Done");
                assert_eq!(detail[1].status, "Pending");
            }
            _ => panic!("expected JobDetailChanged"),
        }
    }

    #[test]
    fn test_chapter_to_embed_deserializes() {
        let event = parse(
            r#"{"type":"ChapterToEmbed","id":7,"chapter":{"url":"u1","idx":0,"title":"Ch1"}}"#,
        );
        match event {
            ServerEvent::ChapterToEmbed { id, .. } => assert_eq!(id, 7),
            _ => panic!("expected ChapterToEmbed"),
        }
    }

    #[test]
    fn test_chapter_embedded_deserializes() {
        let event = parse(r#"{"type":"ChapterEmbedded","id":7,"url":"u1"}"#);
        match event {
            ServerEvent::ChapterEmbedded { id, url } => {
                assert_eq!(id, 7);
                assert_eq!(url, "u1");
            }
            _ => panic!("expected ChapterEmbedded"),
        }
    }

    #[test]
    fn test_chapter_embedded_failed_deserializes() {
        let event = parse(r#"{"type":"ChapterEmbeddedFailed","id":7,"url":"u1"}"#);
        match event {
            ServerEvent::ChapterEmbeddedFailed { id, url } => {
                assert_eq!(id, 7);
                assert_eq!(url, "u1");
            }
            _ => panic!("expected ChapterEmbeddedFailed"),
        }
    }
}
