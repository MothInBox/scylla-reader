use std::sync::Arc;

use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::sse::{Event, KeepAlive, Sse},
};
use base64::Engine;
use futures_util::stream::{Stream, unfold};
use scylla_core::messenger::{AppCommand, AppEvent};
use scylla_core::types::{JobDto, JobOutcomeDto, JobStatus, job_now_ms};
use serde_json::json;

use crate::state::AppState;

/// GET /api/jobs — returns the current job snapshot plus the server's
/// monotonic-relative clock (`server_now_ms`, same epoch as `JobDto` timestamps)
/// so clients can render live elapsed times for running jobs.
pub async fn list_jobs(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let jobs = state.jobs.lock().unwrap();
    Json(json!({
        "server_now_ms": job_now_ms(),
        "jobs": jobs.clone(),
    }))
}

/// GET /api/jobs/stream — SSE stream of app/job events.
pub async fn stream_jobs(
    State(state): State<Arc<AppState>>,
) -> Sse<impl Stream<Item = Result<Event, std::convert::Infallible>>> {
    let rx = state.job_events.subscribe();
    let jobs = state.jobs.clone();
    let stream = unfold(rx, move |mut rx| {
        let jobs = jobs.clone();
        async move {
            loop {
                match rx.recv().await {
                    Ok(event) => {
                        let envelope = event_to_envelope(&event, &jobs);
                        let event = Event::default().json_data(envelope).unwrap();
                        return Some((Ok(event), rx));
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        // Slow consumer — skip missed events and continue.
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
                }
            }
        }
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}

/// POST /api/jobs/cancel — body `{"id": N}`.
pub async fn cancel_job(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<serde_json::Value>,
) -> StatusCode {
    let Some(id) = payload.get("id").and_then(|v| v.as_u64()) else {
        return StatusCode::BAD_REQUEST;
    };
    let _ = state.cmd_tx.send(AppCommand::CancelJob(id));
    StatusCode::OK
}

/// POST /api/jobs/cancel-all.
pub async fn cancel_all(State(state): State<Arc<AppState>>) -> StatusCode {
    let _ = state.cmd_tx.send(AppCommand::CancelAll);
    StatusCode::OK
}

/// POST /api/jobs/retry — body `{"id": N}`.
pub async fn retry_job(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<serde_json::Value>,
) -> StatusCode {
    let Some(id) = payload.get("id").and_then(|v| v.as_u64()) else {
        return StatusCode::BAD_REQUEST;
    };
    let _ = state.cmd_tx.send(AppCommand::RetryJob(id));
    StatusCode::OK
}

/// POST /api/jobs/retry-all.
pub async fn retry_all(State(state): State<Arc<AppState>>) -> StatusCode {
    let _ = state.cmd_tx.send(AppCommand::RetryAllFailed);
    StatusCode::OK
}

/// POST /api/jobs/flush-completed — also clears completed/cancelled snapshot entries.
pub async fn flush_completed(State(state): State<Arc<AppState>>) -> StatusCode {
    let _ = state.cmd_tx.send(AppCommand::FlushCompleted);
    let mut jobs = state.jobs.lock().unwrap();
    jobs.retain(|j| j.status != "Completed" && j.status != "Cancelled");
    StatusCode::OK
}

/// POST /api/jobs/flush-all — also clears the whole snapshot.
pub async fn flush_all(State(state): State<Arc<AppState>>) -> StatusCode {
    let _ = state.cmd_tx.send(AppCommand::FlushAll);
    state.jobs.lock().unwrap().clear();
    StatusCode::OK
}

/// POST /api/jobs/workers — body `{"max_workers": N}`.
pub async fn set_workers(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<serde_json::Value>,
) -> StatusCode {
    let Some(n) = payload.get("max_workers").and_then(|v| v.as_u64()) else {
        return StatusCode::BAD_REQUEST;
    };
    if n == 0 || n > u8::MAX as u64 {
        return StatusCode::BAD_REQUEST;
    }
    let n = n as u8;
    *state.max_workers.lock().unwrap() = n;
    let _ = state.cmd_tx.send(AppCommand::SetMaxWorkers(n));
    StatusCode::OK
}

/// POST /api/jobs/rate-limit — body `{"rate_limit": N}`.
pub async fn set_rate_limit(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<serde_json::Value>,
) -> StatusCode {
    let Some(n) = payload.get("rate_limit").and_then(|v| v.as_u64()) else {
        return StatusCode::BAD_REQUEST;
    };
    *state.rate_limit.lock().unwrap() = n;
    let _ = state.cmd_tx.send(AppCommand::SetRateLimit(n));
    StatusCode::OK
}

/// POST /api/jobs/enqueue — body `{"kind": "Scrape"|"FetchChapter"|"FetchCover"|"EmbedBatch",
/// "url": "...", "chapter_idx": N?, "chapters": [{"url","idx","title"}, ...]}`.
pub async fn enqueue_job(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<serde_json::Value>,
) -> StatusCode {
    let kind = payload.get("kind").and_then(|v| v.as_str()).unwrap_or("");
    match kind {
        "Scrape" => {
            let url = payload.get("url").and_then(|v| v.as_str()).unwrap_or("");
            if url.is_empty() {
                return StatusCode::BAD_REQUEST;
            }
            let _ = state.cmd_tx.send(AppCommand::Scrape(url.to_string()));
            StatusCode::ACCEPTED
        }
        "FetchChapter" => {
            let url = payload.get("url").and_then(|v| v.as_str()).unwrap_or("");
            if url.is_empty() {
                return StatusCode::BAD_REQUEST;
            }
            let Some(idx) = payload.get("chapter_idx").and_then(|v| v.as_u64()) else {
                return StatusCode::BAD_REQUEST;
            };
            let _ = state
                .cmd_tx
                .send(AppCommand::FetchChapter(url.to_string(), idx as usize));
            StatusCode::ACCEPTED
        }
        "FetchCover" => {
            let url = payload.get("url").and_then(|v| v.as_str()).unwrap_or("");
            if url.is_empty() {
                return StatusCode::BAD_REQUEST;
            }
            let _ = state.cmd_tx.send(AppCommand::FetchCover(url.to_string()));
            StatusCode::ACCEPTED
        }
        "EmbedBatch" => {
            let Some(chapters) = payload.get("chapters").and_then(|v| v.as_array()) else {
                return StatusCode::BAD_REQUEST;
            };
            if chapters.is_empty() {
                return StatusCode::BAD_REQUEST;
            }
            let mut refs = Vec::with_capacity(chapters.len());
            for ch in chapters {
                let Some(url) = ch.get("url").and_then(|v| v.as_str()) else {
                    return StatusCode::BAD_REQUEST;
                };
                let Some(idx) = ch.get("idx").and_then(|v| v.as_u64()) else {
                    return StatusCode::BAD_REQUEST;
                };
                let title = ch.get("title").and_then(|v| v.as_str()).unwrap_or("");
                refs.push(scylla_core::types::ChapterRef {
                    url: url.to_string(),
                    idx: idx as usize,
                    title: title.to_string(),
                });
            }
            let _ = state.cmd_tx.send(AppCommand::EmbedBatch(refs));
            StatusCode::ACCEPTED
        }
        _ => StatusCode::BAD_REQUEST,
    }
}

/// Maintains the job snapshot from the event stream.
///
/// Called by the event consumer thread for every `AppEvent` before it is
/// forwarded to the broadcast channel. Returns an optional `JobStatusChanged`
/// event to emit (e.g. when an EmbedBatch job's chapters all finish).
pub fn update_job_snapshot(
    jobs: &Arc<std::sync::Mutex<Vec<JobDto>>>,
    event: &AppEvent,
) -> Option<AppEvent> {
    let mut jobs = jobs.lock().unwrap();
    match event {
        AppEvent::JobEnqueued(job) => {
            jobs.push(JobDto::from(job));
            None
        }
        AppEvent::JobStatusChanged(id, status) => {
            if let Some(job) = jobs.iter_mut().find(|j| j.id == *id) {
                job.status = status.as_str().to_string();
                match status {
                    JobStatus::Running => job.started_at_ms = Some(job_now_ms()),
                    JobStatus::Completed => {
                        job.completed_at_ms = Some(job_now_ms());
                        job.error = None;
                    }
                    JobStatus::Failed(e) => {
                        job.completed_at_ms = Some(job_now_ms());
                        job.error = Some(e.clone());
                    }
                    JobStatus::Queued | JobStatus::Cancelled => {}
                }
            }
            None
        }
        AppEvent::JobOutcome(id, outcome) => {
            if let Some(job) = jobs.iter_mut().find(|j| j.id == *id) {
                job.outcome = Some(JobOutcomeDto::from(outcome));
            }
            None
        }
        AppEvent::JobDetailChanged(id, detail) => {
            if let Some(job) = jobs.iter_mut().find(|j| j.id == *id) {
                let existing = job.detail.get_or_insert_with(Vec::new);
                for incoming in detail {
                    match existing.iter_mut().find(|c| c.url == incoming.url) {
                        Some(existing_ch) => {
                            // Keep "Embedded"/"Failed" statuses set by the
                            // embedding thread — the worker's detail only knows
                            // about the fetch, so it must not clobber them.
                            if existing_ch.status != "Embedded" && existing_ch.status != "Failed" {
                                existing_ch.status = incoming.status.clone();
                            }
                        }
                        None => existing.push(incoming.clone()),
                    }
                }
            }
            None
        }
        AppEvent::ChapterEmbedded(id, url) => {
            if let Some(job) = jobs.iter_mut().find(|j| j.id == *id)
                && let Some(detail) = job.detail.as_mut()
                && let Some(ch) = detail.iter_mut().find(|c| c.url == *url)
            {
                ch.status = "Embedded".into();
            }
            maybe_complete(&mut jobs, *id)
        }
        AppEvent::ChapterEmbeddedFailed(id, url) => {
            if let Some(job) = jobs.iter_mut().find(|j| j.id == *id)
                && let Some(detail) = job.detail.as_mut()
                && let Some(ch) = detail.iter_mut().find(|c| c.url == *url)
            {
                ch.status = "Failed".into();
            }
            maybe_complete(&mut jobs, *id)
        }
        _ => None,
    }
}

/// Whether every chapter in the detail is Embedded or Failed (i.e. no chapter
/// is still Pending or Fetched).
pub fn all_chapters_finished(detail: &[scylla_core::types::ChapterDetail]) -> bool {
    detail
        .iter()
        .all(|c| c.status == "Embedded" || c.status == "Failed")
}

/// If the job's detail is non-empty and every chapter is finished, mark the job
/// Completed and return the `JobStatusChanged` event to emit.
fn maybe_complete(jobs: &mut [JobDto], id: u64) -> Option<AppEvent> {
    let job = jobs.iter_mut().find(|j| j.id == id)?;
    let detail = job.detail.as_ref()?;
    if detail.is_empty() || !all_chapters_finished(detail) {
        return None;
    }
    job.status = "Completed".into();
    job.completed_at_ms = Some(job_now_ms());
    Some(AppEvent::JobStatusChanged(id, JobStatus::Completed))
}

/// Serializes an `AppEvent` into the SSE envelope JSON. The jobs snapshot is
/// consulted for `JobStatusChanged` so the envelope carries the server-stamped
/// `started_at_ms`/`completed_at_ms` (the TUI renders relative times against
/// the same monotonic clock as the snapshot's `server_now_ms`).
pub fn event_to_envelope(
    event: &AppEvent,
    jobs: &Arc<std::sync::Mutex<Vec<JobDto>>>,
) -> serde_json::Value {
    match event {
        AppEvent::JobEnqueued(job) => json!({
            "type": "JobEnqueued",
            "job": JobDto::from(job),
        }),
        AppEvent::JobStatusChanged(id, status) => {
            let error = match status {
                JobStatus::Failed(e) => Some(e.clone()),
                _ => None,
            };
            let (started_at_ms, completed_at_ms) = jobs
                .lock()
                .unwrap()
                .iter()
                .find(|j| j.id == *id)
                .map(|j| (j.started_at_ms, j.completed_at_ms))
                .unwrap_or((None, None));
            json!({
                "type": "JobStatusChanged",
                "id": id,
                "status": status.as_str(),
                "error": error,
                "started_at_ms": started_at_ms,
                "completed_at_ms": completed_at_ms,
            })
        }
        AppEvent::JobOutcome(id, outcome) => json!({
            "type": "JobOutcome",
            "id": id,
            "outcome": JobOutcomeDto::from(outcome),
        }),
        AppEvent::JobDetailChanged(id, detail) => json!({
            "type": "JobDetailChanged",
            "id": id,
            "detail": detail,
        }),
        AppEvent::WorkersChanged(n) => json!({
            "type": "WorkersChanged",
            "max_workers": n,
        }),
        AppEvent::BookScraped(book) => json!({
            "type": "BookScraped",
            "book": book,
        }),
        AppEvent::ChapterFetched(c) => json!({
            "type": "ChapterFetched",
            "chapter": {
                "url": c.url,
                "chapter_idx": c.chapter_idx,
                "title": c.title,
                "content": c.content,
            },
        }),
        AppEvent::ChapterToEmbed(id, c) => json!({
            "type": "ChapterToEmbed",
            "id": id,
            "chapter": {
                "url": c.url,
                "chapter_idx": c.chapter_idx,
                "title": c.title,
                "content": c.content,
            },
        }),
        AppEvent::ChapterEmbedded(id, url) => json!({
            "type": "ChapterEmbedded",
            "id": id,
            "url": url,
        }),
        AppEvent::ChapterEmbeddedFailed(id, url) => json!({
            "type": "ChapterEmbeddedFailed",
            "id": id,
            "url": url,
        }),
        AppEvent::ChapterFetchFailed => json!({ "type": "ChapterFetchFailed" }),
        AppEvent::CoverFetched(url, bytes) => json!({
            "type": "CoverFetched",
            "url": url,
            "bytes": base64::engine::general_purpose::STANDARD.encode(bytes),
        }),
        AppEvent::PluginInstalled(domain, path) => json!({
            "type": "PluginInstalled",
            "domain": domain,
            "path": path,
        }),
        AppEvent::PluginInstallFailed(message) => json!({
            "type": "PluginInstallFailed",
            "message": message,
        }),
    }
}

/// Reconstructs the scraper registry when a plugin is installed, so newly
/// installed plugins are picked up without a restart.
///
/// Returns `true` if the registry was reloaded.
pub fn reload_registry_on_plugin_installed(
    registry: &Arc<std::sync::Mutex<scylla_core::scraper::ScraperRegistry>>,
    event: &AppEvent,
) -> bool {
    if matches!(event, AppEvent::PluginInstalled(..))
        && let Ok(mut reg) = registry.lock()
    {
        *reg = scylla_core::scraper::ScraperRegistry::new();
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use scylla_core::types::{Book, BookStatus, Job, JobKind, JobOutcome, JobPriority};

    fn sample_job(id: u64) -> Job {
        Job::new(
            id,
            JobKind::Scrape("http://example.com".into()),
            "http://example.com".into(),
            JobPriority::Normal,
        )
    }

    fn empty_jobs() -> Arc<std::sync::Mutex<Vec<JobDto>>> {
        Arc::new(std::sync::Mutex::new(Vec::new()))
    }

    #[test]
    fn test_envelope_job_enqueued() {
        let envelope = event_to_envelope(&AppEvent::JobEnqueued(sample_job(1)), &empty_jobs());
        assert_eq!(envelope["type"], "JobEnqueued");
        assert_eq!(envelope["job"]["id"], 1);
        assert_eq!(envelope["job"]["kind"], "Scrape");
        assert_eq!(envelope["job"]["status"], "Queued");
    }

    #[test]
    fn test_envelope_job_status_changed() {
        let envelope = event_to_envelope(
            &AppEvent::JobStatusChanged(2, JobStatus::Running),
            &empty_jobs(),
        );
        assert_eq!(envelope["type"], "JobStatusChanged");
        assert_eq!(envelope["id"], 2);
        assert_eq!(envelope["status"], "Running");
        assert!(envelope["error"].is_null());
        assert!(envelope["started_at_ms"].is_null());
        assert!(envelope["completed_at_ms"].is_null());
    }

    #[test]
    fn test_envelope_job_status_changed_failed_includes_error() {
        let envelope = event_to_envelope(
            &AppEvent::JobStatusChanged(2, JobStatus::Failed("boom".into())),
            &empty_jobs(),
        );
        assert_eq!(envelope["type"], "JobStatusChanged");
        assert_eq!(envelope["id"], 2);
        assert_eq!(envelope["status"], "Failed");
        assert_eq!(envelope["error"], "boom");
    }

    #[test]
    fn test_envelope_job_status_changed_includes_server_stamps() {
        let jobs = empty_jobs();
        jobs.lock().unwrap().push(JobDto {
            id: 2,
            kind: "Scrape".into(),
            status: "Running".into(),
            target: "http://example.com".into(),
            priority: "Normal".into(),
            chapter_idx: None,
            created_at_ms: 100,
            started_at_ms: Some(200),
            completed_at_ms: Some(300),
            error: None,
            outcome: None,
            detail: None,
        });
        let envelope =
            event_to_envelope(&AppEvent::JobStatusChanged(2, JobStatus::Completed), &jobs);
        assert_eq!(envelope["type"], "JobStatusChanged");
        assert_eq!(envelope["id"], 2);
        assert_eq!(envelope["status"], "Completed");
        assert_eq!(envelope["started_at_ms"], 200);
        assert_eq!(envelope["completed_at_ms"], 300);
    }

    #[test]
    fn test_envelope_job_outcome() {
        let outcome = JobOutcome::BookScraped {
            title: "Book".into(),
            chapters: 5,
            cover: true,
        };
        let envelope = event_to_envelope(&AppEvent::JobOutcome(3, outcome), &empty_jobs());
        assert_eq!(envelope["type"], "JobOutcome");
        assert_eq!(envelope["id"], 3);
        assert_eq!(envelope["outcome"]["kind"], "BookScraped");
        assert_eq!(envelope["outcome"]["title"], "Book");
        assert_eq!(envelope["outcome"]["chapters"], 5);
    }

    #[test]
    fn test_envelope_workers_changed() {
        let envelope = event_to_envelope(&AppEvent::WorkersChanged(6), &empty_jobs());
        assert_eq!(envelope["type"], "WorkersChanged");
        assert_eq!(envelope["max_workers"], 6);
    }

    #[test]
    fn test_envelope_book_scraped() {
        let book = Book {
            title: "Book".into(),
            url: "http://example.com".into(),
            status: BookStatus::Reading,
            sessions: vec![],
            active_session_id: None,
            tags: vec![],
            cover_url: None,
            description: None,
            chapters: vec![],
        };
        let envelope = event_to_envelope(&AppEvent::BookScraped(book), &empty_jobs());
        assert_eq!(envelope["type"], "BookScraped");
        assert_eq!(envelope["book"]["title"], "Book");
    }

    #[test]
    fn test_envelope_chapter_fetched() {
        let content = scylla_core::messenger::ChapterContent {
            url: "http://example.com/ch1".into(),
            chapter_idx: 0,
            title: "Ch1".into(),
            content: "text".into(),
        };
        let envelope = event_to_envelope(&AppEvent::ChapterFetched(content), &empty_jobs());
        assert_eq!(envelope["type"], "ChapterFetched");
        assert_eq!(envelope["chapter"]["url"], "http://example.com/ch1");
        assert_eq!(envelope["chapter"]["chapter_idx"], 0);
        assert_eq!(envelope["chapter"]["title"], "Ch1");
        assert_eq!(envelope["chapter"]["content"], "text");
    }

    #[test]
    fn test_envelope_chapter_fetch_failed() {
        let envelope = event_to_envelope(&AppEvent::ChapterFetchFailed, &empty_jobs());
        assert_eq!(envelope["type"], "ChapterFetchFailed");
    }

    #[test]
    fn test_envelope_chapter_to_embed() {
        let content = scylla_core::messenger::ChapterContent {
            url: "http://example.com/ch1".into(),
            chapter_idx: 0,
            title: "Ch1".into(),
            content: "text".into(),
        };
        let envelope = event_to_envelope(&AppEvent::ChapterToEmbed(7, content), &empty_jobs());
        assert_eq!(envelope["type"], "ChapterToEmbed");
        assert_eq!(envelope["id"], 7);
        assert_eq!(envelope["chapter"]["url"], "http://example.com/ch1");
        assert_eq!(envelope["chapter"]["chapter_idx"], 0);
        assert_eq!(envelope["chapter"]["title"], "Ch1");
        assert_eq!(envelope["chapter"]["content"], "text");
    }

    #[test]
    fn test_envelope_chapter_embedded() {
        let envelope = event_to_envelope(
            &AppEvent::ChapterEmbedded(7, "http://example.com/ch1".into()),
            &empty_jobs(),
        );
        assert_eq!(envelope["type"], "ChapterEmbedded");
        assert_eq!(envelope["id"], 7);
        assert_eq!(envelope["url"], "http://example.com/ch1");
    }

    #[test]
    fn test_envelope_plugin_installed() {
        let envelope = event_to_envelope(
            &AppEvent::PluginInstalled("example.com".into(), "/p.wasm".into()),
            &empty_jobs(),
        );
        assert_eq!(envelope["type"], "PluginInstalled");
        assert_eq!(envelope["domain"], "example.com");
        assert_eq!(envelope["path"], "/p.wasm");
    }

    #[test]
    fn test_envelope_plugin_install_failed() {
        let envelope =
            event_to_envelope(&AppEvent::PluginInstallFailed("boom".into()), &empty_jobs());
        assert_eq!(envelope["type"], "PluginInstallFailed");
        assert_eq!(envelope["message"], "boom");
    }

    #[test]
    fn test_envelope_cover_fetched_base64() {
        let envelope = event_to_envelope(
            &AppEvent::CoverFetched("http://example.com/cover.jpg".into(), vec![1, 2, 3]),
            &empty_jobs(),
        );
        assert_eq!(envelope["type"], "CoverFetched");
        assert_eq!(envelope["url"], "http://example.com/cover.jpg");
        assert_eq!(envelope["bytes"], "AQID");
    }

    #[test]
    fn test_update_job_snapshot_lifecycle() {
        let jobs: Arc<std::sync::Mutex<Vec<JobDto>>> = Arc::new(std::sync::Mutex::new(Vec::new()));

        update_job_snapshot(&jobs, &AppEvent::JobEnqueued(sample_job(1)));
        update_job_snapshot(&jobs, &AppEvent::JobStatusChanged(1, JobStatus::Running));
        update_job_snapshot(
            &jobs,
            &AppEvent::JobOutcome(
                1,
                JobOutcome::BookScraped {
                    title: "Book".into(),
                    chapters: 2,
                    cover: false,
                },
            ),
        );
        update_job_snapshot(&jobs, &AppEvent::JobStatusChanged(1, JobStatus::Completed));

        let snapshot = jobs.lock().unwrap();
        assert_eq!(snapshot.len(), 1);
        let job = &snapshot[0];
        assert_eq!(job.status, "Completed");
        assert!(job.started_at_ms.is_some());
        assert!(job.completed_at_ms.is_some());
        let outcome = job.outcome.as_ref().unwrap();
        assert_eq!(outcome.kind, "BookScraped");
        assert_eq!(outcome.chapters, Some(2));
    }

    #[test]
    fn test_update_job_snapshot_failed_sets_error() {
        let jobs: Arc<std::sync::Mutex<Vec<JobDto>>> = Arc::new(std::sync::Mutex::new(Vec::new()));

        update_job_snapshot(&jobs, &AppEvent::JobEnqueued(sample_job(2)));
        update_job_snapshot(
            &jobs,
            &AppEvent::JobStatusChanged(2, JobStatus::Failed("boom".into())),
        );

        let snapshot = jobs.lock().unwrap();
        let job = &snapshot[0];
        assert_eq!(job.status, "Failed");
        assert_eq!(job.error.as_deref(), Some("boom"));
        assert!(job.completed_at_ms.is_some());
    }

    #[test]
    fn test_reload_registry_on_plugin_installed() {
        let registry: Arc<std::sync::Mutex<scylla_core::scraper::ScraperRegistry>> = Arc::new(
            std::sync::Mutex::new(scylla_core::scraper::ScraperRegistry::new()),
        );

        // Non-plugin events do not trigger a reload.
        assert!(!reload_registry_on_plugin_installed(
            &registry,
            &AppEvent::JobEnqueued(sample_job(1)),
        ));
        assert!(!reload_registry_on_plugin_installed(
            &registry,
            &AppEvent::PluginInstallFailed("boom".into()),
        ));

        // PluginInstalled triggers a reload (registry is reconstructed).
        assert!(reload_registry_on_plugin_installed(
            &registry,
            &AppEvent::PluginInstalled("example.com".into(), "/p.wasm".into()),
        ));
        assert!(registry.lock().is_ok());
    }

    #[test]
    fn test_envelope_job_detail_changed() {
        let detail = vec![scylla_core::types::ChapterDetail {
            title: "Ch1".into(),
            url: "http://example.com/ch1".into(),
            status: "Done".into(),
        }];
        let envelope = event_to_envelope(&AppEvent::JobDetailChanged(3, detail), &empty_jobs());
        assert_eq!(envelope["type"], "JobDetailChanged");
        assert_eq!(envelope["id"], 3);
        assert_eq!(envelope["detail"][0]["status"], "Done");
        assert_eq!(envelope["detail"][0]["title"], "Ch1");
    }

    #[test]
    fn test_update_job_snapshot_job_detail_changed() {
        let jobs: Arc<std::sync::Mutex<Vec<JobDto>>> = Arc::new(std::sync::Mutex::new(Vec::new()));

        update_job_snapshot(&jobs, &AppEvent::JobEnqueued(sample_job(4)));
        let detail = vec![scylla_core::types::ChapterDetail {
            title: "Ch1".into(),
            url: "http://example.com/ch1".into(),
            status: "Done".into(),
        }];
        update_job_snapshot(&jobs, &AppEvent::JobDetailChanged(4, detail));

        let snapshot = jobs.lock().unwrap();
        let job = &snapshot[0];
        let detail = job.detail.as_ref().unwrap();
        assert_eq!(detail.len(), 1);
        assert_eq!(detail[0].status, "Done");
    }

    #[test]
    fn test_update_job_snapshot_chapter_embedded_flips_status() {
        let jobs: Arc<std::sync::Mutex<Vec<JobDto>>> = Arc::new(std::sync::Mutex::new(Vec::new()));

        update_job_snapshot(&jobs, &AppEvent::JobEnqueued(sample_job(5)));
        let detail = vec![
            scylla_core::types::ChapterDetail {
                title: "Ch1".into(),
                url: "http://example.com/ch1".into(),
                status: "Fetched".into(),
            },
            scylla_core::types::ChapterDetail {
                title: "Ch2".into(),
                url: "http://example.com/ch2".into(),
                status: "Fetched".into(),
            },
        ];
        update_job_snapshot(&jobs, &AppEvent::JobDetailChanged(5, detail));

        // The embedding thread reports chapter 1 embedded.
        update_job_snapshot(
            &jobs,
            &AppEvent::ChapterEmbedded(5, "http://example.com/ch1".into()),
        );

        let snapshot = jobs.lock().unwrap();
        let job = &snapshot[0];
        let detail = job.detail.as_ref().unwrap();
        assert_eq!(detail[0].status, "Embedded");
        assert_eq!(detail[1].status, "Fetched");
    }

    #[test]
    fn test_job_detail_changed_does_not_clobber_embedded_status() {
        let jobs: Arc<std::sync::Mutex<Vec<JobDto>>> = Arc::new(std::sync::Mutex::new(Vec::new()));

        update_job_snapshot(&jobs, &AppEvent::JobEnqueued(sample_job(8)));
        let detail = vec![
            scylla_core::types::ChapterDetail {
                title: "Ch1".into(),
                url: "http://example.com/ch1".into(),
                status: "Fetched".into(),
            },
            scylla_core::types::ChapterDetail {
                title: "Ch2".into(),
                url: "http://example.com/ch2".into(),
                status: "Fetched".into(),
            },
        ];
        update_job_snapshot(&jobs, &AppEvent::JobDetailChanged(8, detail));

        // The embedding thread reports chapter 1 embedded.
        update_job_snapshot(
            &jobs,
            &AppEvent::ChapterEmbedded(8, "http://example.com/ch1".into()),
        );

        // The worker emits another JobDetailChanged with its fetch-only view
        // (ch1 "Fetched", ch2 "Fetched") — it must not clobber ch1's "Embedded".
        let worker_detail = vec![
            scylla_core::types::ChapterDetail {
                title: "Ch1".into(),
                url: "http://example.com/ch1".into(),
                status: "Fetched".into(),
            },
            scylla_core::types::ChapterDetail {
                title: "Ch2".into(),
                url: "http://example.com/ch2".into(),
                status: "Fetched".into(),
            },
        ];
        update_job_snapshot(&jobs, &AppEvent::JobDetailChanged(8, worker_detail));

        let snapshot = jobs.lock().unwrap();
        let job = &snapshot[0];
        let detail = job.detail.as_ref().unwrap();
        assert_eq!(detail[0].status, "Embedded");
        assert_eq!(detail[1].status, "Fetched");
    }

    #[test]
    fn test_all_chapters_finished() {
        let detail = |statuses: &[&str]| {
            statuses
                .iter()
                .map(|s| scylla_core::types::ChapterDetail {
                    title: "Ch".into(),
                    url: "u".into(),
                    status: s.to_string(),
                })
                .collect::<Vec<_>>()
        };
        assert!(all_chapters_finished(&detail(&["Embedded", "Embedded"])));
        assert!(all_chapters_finished(&detail(&["Embedded", "Failed"])));
        assert!(all_chapters_finished(&detail(&["Failed", "Failed"])));
        assert!(!all_chapters_finished(&detail(&["Embedded", "Fetched"])));
        assert!(!all_chapters_finished(&detail(&["Pending", "Embedded"])));
        assert!(all_chapters_finished(&[]));
    }

    #[test]
    fn test_embed_batch_completes_when_all_chapters_finished() {
        let jobs: Arc<std::sync::Mutex<Vec<JobDto>>> = Arc::new(std::sync::Mutex::new(Vec::new()));

        update_job_snapshot(&jobs, &AppEvent::JobEnqueued(sample_job(6)));
        update_job_snapshot(&jobs, &AppEvent::JobStatusChanged(6, JobStatus::Running));
        let detail = vec![
            scylla_core::types::ChapterDetail {
                title: "Ch1".into(),
                url: "http://example.com/ch1".into(),
                status: "Fetched".into(),
            },
            scylla_core::types::ChapterDetail {
                title: "Ch2".into(),
                url: "http://example.com/ch2".into(),
                status: "Fetched".into(),
            },
        ];
        update_job_snapshot(&jobs, &AppEvent::JobDetailChanged(6, detail));

        // One chapter embedded, one still fetched → job stays Running.
        let emit = update_job_snapshot(
            &jobs,
            &AppEvent::ChapterEmbedded(6, "http://example.com/ch1".into()),
        );
        assert!(emit.is_none());
        {
            let snapshot = jobs.lock().unwrap();
            assert_eq!(snapshot[0].status, "Running");
        }

        // Second chapter embedded → all finished → job Completed + event emitted.
        let emit = update_job_snapshot(
            &jobs,
            &AppEvent::ChapterEmbedded(6, "http://example.com/ch2".into()),
        );
        assert!(matches!(
            emit,
            Some(AppEvent::JobStatusChanged(6, JobStatus::Completed))
        ));
        let snapshot = jobs.lock().unwrap();
        assert_eq!(snapshot[0].status, "Completed");
        assert!(snapshot[0].completed_at_ms.is_some());
    }

    #[test]
    fn test_embed_batch_completes_with_failed_chapter() {
        let jobs: Arc<std::sync::Mutex<Vec<JobDto>>> = Arc::new(std::sync::Mutex::new(Vec::new()));

        update_job_snapshot(&jobs, &AppEvent::JobEnqueued(sample_job(7)));
        let detail = vec![
            scylla_core::types::ChapterDetail {
                title: "Ch1".into(),
                url: "http://example.com/ch1".into(),
                status: "Fetched".into(),
            },
            scylla_core::types::ChapterDetail {
                title: "Ch2".into(),
                url: "http://example.com/ch2".into(),
                status: "Fetched".into(),
            },
        ];
        update_job_snapshot(&jobs, &AppEvent::JobDetailChanged(7, detail));

        // Chapter 1 embedded, chapter 2 failed to embed → all finished.
        update_job_snapshot(
            &jobs,
            &AppEvent::ChapterEmbedded(7, "http://example.com/ch1".into()),
        );
        let emit = update_job_snapshot(
            &jobs,
            &AppEvent::ChapterEmbeddedFailed(7, "http://example.com/ch2".into()),
        );
        assert!(matches!(
            emit,
            Some(AppEvent::JobStatusChanged(7, JobStatus::Completed))
        ));
        let snapshot = jobs.lock().unwrap();
        assert_eq!(snapshot[0].status, "Completed");
        let detail = snapshot[0].detail.as_ref().unwrap();
        assert_eq!(detail[0].status, "Embedded");
        assert_eq!(detail[1].status, "Failed");
    }

    #[test]
    fn test_envelope_chapter_embedded_failed() {
        let envelope = event_to_envelope(
            &AppEvent::ChapterEmbeddedFailed(7, "http://example.com/ch1".into()),
            &empty_jobs(),
        );
        assert_eq!(envelope["type"], "ChapterEmbeddedFailed");
        assert_eq!(envelope["id"], 7);
        assert_eq!(envelope["url"], "http://example.com/ch1");
    }
}
