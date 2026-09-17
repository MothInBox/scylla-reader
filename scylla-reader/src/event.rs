use crate::event_types::{ChapterGroup, ChapterHit, ServerEvent};
use crate::state::AppState;
use crate::state::modal::{Modal, SearchStatus};
use std::sync::mpsc;

pub fn drain_events(state: &mut AppState, event_rx: &mpsc::Receiver<ServerEvent>) {
    while let Ok(event) = event_rx.try_recv() {
        match event {
            ServerEvent::JobsSnapshot {
                jobs,
                server_now_ms,
            } => {
                crate::settings::log(
                    crate::settings::LogLevel::Debug,
                    "UI",
                    &format!("Jobs snapshot received: {} jobs", jobs.len()),
                );
                state.jobs.jobs = jobs;
                state.jobs.server_now_ms = server_now_ms;
                state.jobs.active_count = state
                    .jobs
                    .jobs
                    .iter()
                    .filter(|j| j.status == "Running")
                    .count() as u8;
                // Clamp selection/detail to the new list so a shrinking
                // snapshot never leaves a dangling index.
                let filtered_len = state.jobs.filtered_jobs().len();
                if state.jobs.selected >= filtered_len {
                    state.jobs.selected = filtered_len.saturating_sub(1);
                }
                if let Some(idx) = state.jobs.detail_expanded
                    && idx >= state.jobs.jobs.len()
                {
                    state.jobs.detail_expanded = None;
                }
            }
            ServerEvent::JobEnqueued { job } => {
                crate::settings::log(
                    crate::settings::LogLevel::Debug,
                    "UI",
                    &format!("Job enqueued: {} ({})", job.target, job.id),
                );
                // Dedupe against the snapshot — the same job may arrive both
                // via the initial snapshot and the live stream.
                if !state.jobs.jobs.iter().any(|j| j.id == job.id) {
                    state.jobs.jobs.push(job);
                }
            }
            ServerEvent::JobStatusChanged {
                id,
                status,
                error,
                started_at_ms,
                completed_at_ms,
            } => {
                crate::settings::log(
                    crate::settings::LogLevel::Debug,
                    "UI",
                    &format!("Job {} status: {}", id, status),
                );
                state.jobs.update_from_status(
                    id,
                    &status,
                    error.as_deref(),
                    started_at_ms,
                    completed_at_ms,
                );
                state.jobs.active_count = state
                    .jobs
                    .jobs
                    .iter()
                    .filter(|j| j.status == "Running")
                    .count() as u8;
            }
            ServerEvent::JobOutcome { id, outcome } => {
                crate::settings::log(
                    crate::settings::LogLevel::Debug,
                    "UI",
                    &format!("Job {} outcome", id),
                );
                state.jobs.set_outcome(id, outcome);
            }
            ServerEvent::JobDetailChanged { id, detail } => {
                crate::settings::log(
                    crate::settings::LogLevel::Debug,
                    "UI",
                    &format!("Job {} detail: {} chapters", id, detail.len()),
                );
                state.jobs.update_from_detail(id, detail);
            }
            ServerEvent::ChapterToEmbed { .. } => {
                // Server-internal — the TUI ignores it.
                crate::settings::log(
                    crate::settings::LogLevel::Debug,
                    "UI",
                    "ChapterToEmbed event ignored",
                );
            }
            ServerEvent::WorkersChanged { max_workers } => {
                crate::settings::log(
                    crate::settings::LogLevel::Debug,
                    "UI",
                    &format!("Workers changed to {}", max_workers),
                );
                state.jobs.max_workers = max_workers;
                state.lib.settings.max_workers = max_workers;
            }
            ServerEvent::BookScraped { book } => {
                crate::settings::log(
                    crate::settings::LogLevel::Debug,
                    "UI",
                    &format!("UI received book: {}", book.title),
                );
                let url = book.url.clone();
                if let Some(existing) = state.lib.library.books.iter_mut().find(|b| b.url == url) {
                    existing.title = book.title.clone();
                    existing.cover_url = book.cover_url.clone();
                    existing.description = book.description.clone();
                    existing.chapters = book.chapters.clone();
                } else {
                    state.lib.library.books.push(book);
                }

                // Persist the MERGED library book — never the raw scraped one,
                // so user data (status, tags, sessions) is preserved.
                let mut merged = match state
                    .lib
                    .library
                    .books
                    .iter()
                    .find(|b| b.url == url)
                    .cloned()
                {
                    Some(book) => book,
                    None => continue,
                };
                if let Some(backend) = state.lib.manager.primary_backend() {
                    if let Err(e) = crate::storage::client::block_on(backend.upsert_book(&merged)) {
                        crate::settings::log(
                            crate::settings::LogLevel::Error,
                            "UI",
                            &format!("Failed to persist book {}: {}", url, e),
                        );
                    }
                    match crate::storage::client::block_on(backend.list_sessions(&merged.url)) {
                        Ok(sessions) => merged.sessions = sessions,
                        Err(e) => crate::settings::log(
                            crate::settings::LogLevel::Error,
                            "UI",
                            &format!("Failed to list sessions for {}: {}", url, e),
                        ),
                    }
                    // The sessions response doesn't carry the active session, so
                    // refresh it from the server's book state.
                    match crate::storage::client::block_on(backend.get_book(&merged.url)) {
                        Ok(server_book) => merged.active_session_id = server_book.active_session_id,
                        Err(e) => crate::settings::log(
                            crate::settings::LogLevel::Error,
                            "UI",
                            &format!("Failed to fetch book {}: {}", url, e),
                        ),
                    }
                }
                if let Some(existing) = state.lib.library.books.iter_mut().find(|b| b.url == url) {
                    existing.sessions = merged.sessions;
                    existing.active_session_id = merged.active_session_id;
                }
            }
            ServerEvent::ChapterFetched {
                chapter_idx,
                title,
                content,
                ..
            } => {
                crate::settings::log(
                    crate::settings::LogLevel::Debug,
                    "UI",
                    &format!("Chapter received: {}", title),
                );
                if !state.reader.loading {
                    // Background fetch (embed modal / crawl) — the user isn't
                    // waiting for this chapter, so don't navigate to the reader
                    // or touch session progress.
                    crate::settings::log(
                        crate::settings::LogLevel::Debug,
                        "UI",
                        "Background chapter fetch — skipping reader navigation",
                    );
                    continue;
                }
                let session_id = state.reader.session_id;
                if let Some(book) = state.lib.library.selected_book_mut()
                    && let Some(session) = book.sessions.iter_mut().find(|s| s.id == session_id)
                {
                    session.progress.current = chapter_idx as u32;
                }
                // Guest sessions (session_id < 0, e.g. AI-jump to a book not in
                // the library) have no server-side progress row — skip the
                // update to avoid a 404 error log on every chapter.
                if session_id >= 0
                    && let Some(backend) = state.lib.manager.primary_backend()
                    && let Err(e) = crate::storage::client::block_on(
                        backend.update_progress(session_id, chapter_idx as u32),
                    )
                {
                    crate::settings::log(
                        crate::settings::LogLevel::Error,
                        "UI",
                        &format!(
                            "Failed to persist progress for session {}: {}",
                            session_id, e
                        ),
                    );
                }
                state.open_reader_chapter(
                    title,
                    content,
                    chapter_idx,
                    state.reader.session_id,
                    state.reader.session_name.clone(),
                );
            }
            ServerEvent::ChapterFetchFailed => {
                crate::settings::log(
                    crate::settings::LogLevel::Debug,
                    "UI",
                    "Chapter fetch failed",
                );
                state.reader.loading = false;
            }
            ServerEvent::CoverFetched { url, bytes } => {
                crate::settings::log(
                    crate::settings::LogLevel::Debug,
                    "UI",
                    &format!("Cover fetched: {}", url),
                );
                match image::load_from_memory(&bytes) {
                    Ok(img) => {
                        let protocol = state.cover_picker.new_resize_protocol(img);
                        state.lib.library.cover_cache.insert(url, protocol);
                    }
                    Err(e) => crate::settings::log(
                        crate::settings::LogLevel::Error,
                        "UI",
                        &format!("Failed to decode cover image {}: {}", url, e),
                    ),
                }
            }
            ServerEvent::PluginInstalled { domain, path } => {
                crate::settings::log(
                    crate::settings::LogLevel::Debug,
                    "PLUGIN",
                    &format!("Plugin installed: {} at {}", domain, path),
                );
                state.lib.settings.reload_plugins();
            }
            ServerEvent::PluginInstallFailed { message } => {
                crate::settings::log(
                    crate::settings::LogLevel::Error,
                    "PLUGIN",
                    &format!("Plugin install failed: {}", message),
                );
            }
            ServerEvent::ConnectionState { connected } => {
                crate::settings::log(
                    crate::settings::LogLevel::Debug,
                    "UI",
                    &format!(
                        "SSE connection {}",
                        if connected { "established" } else { "lost" }
                    ),
                );
                state.jobs.connected = connected;
                // A dropped SSE means any in-flight chapter fetch will never
                // resolve — don't leave the reader stuck on a spinner.
                if !connected {
                    state.reader.loading = false;
                }
            }
            ServerEvent::AiSearchResults { query, result } => {
                // Only apply when the chapter-results modal is open for the
                // same query; anything else is stale.
                let is_current = matches!(
                    &state.ui.modal,
                    Modal::ChapterResults { query: q, .. } if *q == query
                );
                if !is_current {
                    crate::settings::log(
                        crate::settings::LogLevel::Debug,
                        "AI",
                        &format!("Stale AI search result dropped for query: {}", query),
                    );
                    continue;
                }
                match result {
                    Ok(hits) => {
                        let groups = build_groups(hits);
                        let status = if groups.is_empty() {
                            SearchStatus::Empty
                        } else {
                            SearchStatus::Ready
                        };
                        if let Modal::ChapterResults {
                            groups: g,
                            cursor,
                            scroll_offset,
                            status: s,
                            ..
                        } = &mut state.ui.modal
                        {
                            *g = groups;
                            *cursor = 0;
                            *scroll_offset = 0;
                            *s = status;
                        }
                    }
                    Err(e) => {
                        crate::settings::log(
                            crate::settings::LogLevel::Error,
                            "AI",
                            &format!("AI search failed: {}", e),
                        );
                        if let Modal::ChapterResults { status: s, .. } = &mut state.ui.modal {
                            *s = SearchStatus::Error(e);
                        }
                    }
                }
            }
        }
    }
}

/// Group flat chapter hits by book (first-appearance order), sort each group's
/// chapters by score descending, and order the books by their best chapter's
/// score descending.
fn build_groups(hits: Vec<ChapterHit>) -> Vec<ChapterGroup> {
    let mut groups: Vec<ChapterGroup> = Vec::new();
    for hit in hits {
        if let Some(group) = groups.iter_mut().find(|g| g.book_url == hit.book_url) {
            group.chapters.push(hit);
        } else {
            groups.push(ChapterGroup {
                book_url: hit.book_url.clone(),
                book_title: hit.book_title.clone(),
                genres: hit.genres.clone(),
                chapters: vec![hit],
                best_score: 0.0,
            });
        }
    }
    for group in &mut groups {
        group.chapters.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        group.best_score = group.chapters.first().map(|c| c.score).unwrap_or(0.0);
    }
    groups.sort_by(|a, b| {
        b.best_score
            .partial_cmp(&a.best_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    groups
}

pub fn update_covers(state: &mut AppState, fetched_covers: &mut std::collections::HashSet<String>) {
    let current_cover_url = state
        .lib
        .library
        .selected_book()
        .and_then(|b| b.cover_url.clone());

    if let Some(url) = current_cover_url
        && !fetched_covers.contains(&url)
    {
        fetched_covers.insert(url.clone());
        let base = crate::storage::client::api_base(state);
        if let Err(e) = crate::storage::client::block_on(crate::storage::client::enqueue_job(
            &base,
            "FetchCover",
            &url,
            None,
        )) {
            crate::settings::log(
                crate::settings::LogLevel::Error,
                "UI",
                &format!("Failed to enqueue cover fetch: {}", e),
            );
        }
    }
}

/// How often the selected book's embedding status is re-fetched so the
/// "Embedded: N/M" indicator shows live progress as embed jobs complete.
const EMBEDDING_STATUS_REFRESH: std::time::Duration = std::time::Duration::from_secs(10);

/// Whether the embedding status for a book should be (re-)fetched: never
/// fetched before, or the last fetch is older than [`EMBEDDING_STATUS_REFRESH`].
fn should_refresh(last_fetch: Option<std::time::Instant>, now: std::time::Instant) -> bool {
    match last_fetch {
        None => true,
        Some(last) => now.duration_since(last) >= EMBEDDING_STATUS_REFRESH,
    }
}

/// Fetch the embedding status for the selected book and cache it, so the
/// details panel can show embedding progress. Re-fetches periodically (see
/// [`EMBEDDING_STATUS_REFRESH`]) so the indicator tracks live progress.
/// Failures (e.g. the book isn't embedded yet) are logged at Debug and never
/// panic.
pub fn update_embedding_statuses(state: &mut AppState) {
    let current_url = state.lib.library.selected_book().map(|b| b.url.clone());
    let Some(url) = current_url else {
        return;
    };
    let last_fetch = state
        .lib
        .library
        .embedding_status_fetched_at
        .get(&url)
        .copied();
    if !should_refresh(last_fetch, std::time::Instant::now()) {
        return;
    }
    let base = crate::storage::client::api_base(state);
    match crate::storage::client::block_on(crate::storage::client::embedding_status(&base, &url)) {
        Ok(status) => {
            state
                .lib
                .library
                .embedding_status_cache
                .insert(url.clone(), status);
            state
                .lib
                .library
                .embedding_status_fetched_at
                .insert(url, std::time::Instant::now());
        }
        Err(e) => {
            crate::settings::log(
                crate::settings::LogLevel::Debug,
                "UI",
                &format!("Failed to fetch embedding status for {}: {}", url, e),
            );
            // Record the attempt so we don't hammer a failing endpoint.
            state
                .lib
                .library
                .embedding_status_fetched_at
                .insert(url, std::time::Instant::now());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Page;
    use crate::test_helpers::*;
    use scylla_core::types::{BookStatus, Chapter, JobDto, Progress, Session};

    fn sample_book(url: &str) -> scylla_core::types::Book {
        scylla_core::types::Book {
            title: format!("Book {}", url),
            url: url.to_string(),
            status: BookStatus::Reading,
            sessions: vec![],
            active_session_id: None,
            tags: vec!["tag1".into()],
            cover_url: None,
            description: Some("desc".into()),
            chapters: vec![Chapter {
                url: "ch1".into(),
                title: "Chapter 1".into(),
                order: 1,
            }],
        }
    }

    fn sample_job(id: u64, status: &str) -> JobDto {
        JobDto {
            id,
            kind: "Scrape".into(),
            status: status.into(),
            target: "http://example.com".into(),
            priority: "Normal".into(),
            chapter_idx: None,
            created_at_ms: 0,
            started_at_ms: None,
            completed_at_ms: None,
            error: None,
            outcome: None,
            detail: None,
        }
    }

    fn send(state: &mut AppState, event: ServerEvent) {
        let (tx, rx) = mpsc::channel();
        tx.send(event).unwrap();
        drain_events(state, &rx);
    }

    #[test]
    fn test_book_scraped_persists_merged_book() {
        let mock = MockBackend::new("mock");
        let calls = mock.calls.clone();
        let books = mock.books.clone();
        let mut state = test_state_with_backend(Box::new(mock));

        // Pre-existing library book with user data (Completed + tag + session).
        let mut existing = sample_book("http://example.com/book");
        existing.status = BookStatus::Completed;
        existing.tags = vec!["keep".into()];
        existing.sessions = vec![Session {
            id: 5,
            book_url: "http://example.com/book".into(),
            name: "S5".into(),
            progress: Progress {
                current: 0,
                total: 10,
            },
            created_at: String::new(),
            updated_at: String::new(),
        }];
        state.lib.library.books.push(existing);

        // Raw scraped event carries defaults (Reading, no tags).
        let mut scraped = sample_book("http://example.com/book");
        scraped.status = BookStatus::Reading;
        scraped.tags = vec![];
        scraped.title = "Scraped Title".into();

        send(&mut state, ServerEvent::BookScraped { book: scraped });

        let calls = calls.lock().unwrap().clone();
        assert!(
            calls.iter().any(|c| c == "upsert:http://example.com/book"),
            "expected upsert call, got: {:?}",
            calls
        );
        assert!(
            calls
                .iter()
                .any(|c| c == "list_sessions:http://example.com/book"),
            "expected list_sessions call, got: {:?}",
            calls
        );

        // The upserted book must be the MERGED one: user data preserved.
        let stored = books
            .lock()
            .unwrap()
            .iter()
            .find(|b| b.url == "http://example.com/book")
            .cloned()
            .unwrap();
        assert_eq!(stored.status, BookStatus::Completed);
        assert_eq!(stored.tags, vec!["keep"]);
        assert_eq!(stored.title, "Scraped Title");
    }

    #[test]
    fn test_book_scraped_new_book_is_persisted() {
        let mock = MockBackend::new("mock");
        let calls = mock.calls.clone();
        let mut state = test_state_with_backend(Box::new(mock));

        send(
            &mut state,
            ServerEvent::BookScraped {
                book: sample_book("http://example.com/new"),
            },
        );

        let calls = calls.lock().unwrap().clone();
        assert!(
            calls.iter().any(|c| c == "upsert:http://example.com/new"),
            "expected upsert call, got: {:?}",
            calls
        );
        assert_eq!(state.lib.library.books.len(), 1);
    }

    #[test]
    fn test_book_scraped_refreshes_sessions_from_backend() {
        let mock = MockBackend::new("mock");
        let mut state = test_state_with_backend(Box::new(mock));

        // Pre-existing library book with a stale in-memory session.
        let mut existing = sample_book("http://example.com/book");
        existing.sessions = vec![Session {
            id: 999,
            book_url: "http://example.com/book".into(),
            name: "Stale".into(),
            progress: Progress {
                current: 0,
                total: 10,
            },
            created_at: String::new(),
            updated_at: String::new(),
        }];
        state.lib.library.books.push(existing);

        send(
            &mut state,
            ServerEvent::BookScraped {
                book: sample_book("http://example.com/book"),
            },
        );

        // The mock's upsert created an "Initial" session; list_sessions returns
        // it, and the write-back must replace the stale in-memory sessions.
        let book = state
            .lib
            .library
            .books
            .iter()
            .find(|b| b.url == "http://example.com/book")
            .unwrap();
        assert_eq!(book.sessions.len(), 1);
        assert_eq!(book.sessions[0].name, "Initial");
        assert_ne!(book.sessions[0].id, 999);
    }

    #[test]
    fn test_chapter_fetched_persists_progress() {
        let mock = MockBackend::new("mock");
        let calls = mock.calls.clone();
        let mut state = test_state_with_backend(Box::new(mock));
        state.lib.library.add_book("Book".into(), "url".into());
        if let Some(book) = state.lib.library.books.iter_mut().find(|b| b.url == "url") {
            book.sessions.push(Session {
                id: 42,
                book_url: "url".into(),
                name: "S42".into(),
                progress: Progress {
                    current: 0,
                    total: 10,
                },
                created_at: String::new(),
                updated_at: String::new(),
            });
        }
        state.reader.session_id = 42;
        state.reader.loading = true; // user is waiting for this chapter

        send(
            &mut state,
            ServerEvent::ChapterFetched {
                url: "".into(),
                chapter_idx: 3,
                title: "Ch3".into(),
                content: "text".into(),
            },
        );

        let calls = calls.lock().unwrap().clone();
        assert!(
            calls.iter().any(|c| c == "update_progress:42:3"),
            "expected update_progress call, got: {:?}",
            calls
        );
        assert_eq!(state.ui.page, Page::Reader);
    }

    #[test]
    fn test_chapter_fetched_guest_session_skips_progress_update() {
        let mock = MockBackend::new("mock");
        let calls = mock.calls.clone();
        let mut state = test_state_with_backend(Box::new(mock));
        state.reader.session_id = -1; // guest session (AI jump, book not in library)
        state.reader.loading = true; // user is waiting for this chapter

        send(
            &mut state,
            ServerEvent::ChapterFetched {
                url: "".into(),
                chapter_idx: 3,
                title: "Ch3".into(),
                content: "text".into(),
            },
        );

        let calls = calls.lock().unwrap().clone();
        assert!(
            !calls.iter().any(|c| c.starts_with("update_progress")),
            "expected no update_progress call for guest session, got: {:?}",
            calls
        );
    }

    #[test]
    fn test_chapter_fetched_background_does_not_open_reader() {
        let mock = MockBackend::new("mock");
        let calls = mock.calls.clone();
        let mut state = test_state_with_backend(Box::new(mock));
        state.ui.page = Page::Library;
        state.reader.loading = false; // background fetch (embed modal / crawl)

        send(
            &mut state,
            ServerEvent::ChapterFetched {
                url: "".into(),
                chapter_idx: 3,
                title: "Ch3".into(),
                content: "text".into(),
            },
        );

        // The reader page must NOT open and progress must NOT be updated.
        assert_eq!(state.ui.page, Page::Library);
        let calls = calls.lock().unwrap().clone();
        assert!(
            !calls.iter().any(|c| c.starts_with("update_progress")),
            "expected no update_progress call for background fetch, got: {:?}",
            calls
        );
    }

    #[test]
    fn test_jobs_snapshot_replaces_jobs() {
        let mut state = test_state();
        state.jobs.jobs.push(sample_job(1, "Running"));
        send(
            &mut state,
            ServerEvent::JobsSnapshot {
                jobs: vec![sample_job(2, "Queued"), sample_job(3, "Completed")],
                server_now_ms: 12345,
            },
        );
        assert_eq!(state.jobs.jobs.len(), 2);
        assert_eq!(state.jobs.jobs[0].id, 2);
        assert_eq!(state.jobs.active_count, 0);
        assert_eq!(state.jobs.server_now_ms, 12345);
    }

    #[test]
    fn test_jobs_snapshot_recomputes_active_count() {
        let mut state = test_state();
        send(
            &mut state,
            ServerEvent::JobsSnapshot {
                jobs: vec![sample_job(1, "Running"), sample_job(2, "Running")],
                server_now_ms: 0,
            },
        );
        assert_eq!(state.jobs.active_count, 2);
    }

    #[test]
    fn test_jobs_snapshot_clamps_selection_and_detail() {
        let mut state = test_state();
        state.jobs.jobs.push(sample_job(1, "Queued"));
        state.jobs.jobs.push(sample_job(2, "Queued"));
        state.jobs.selected = 1;
        state.jobs.detail_expanded = Some(1);
        send(
            &mut state,
            ServerEvent::JobsSnapshot {
                jobs: vec![sample_job(3, "Queued")],
                server_now_ms: 0,
            },
        );
        assert_eq!(state.jobs.selected, 0);
        assert_eq!(state.jobs.detail_expanded, None);
    }

    #[test]
    fn test_jobs_snapshot_empty_clamps_selection() {
        let mut state = test_state();
        state.jobs.jobs.push(sample_job(1, "Queued"));
        state.jobs.selected = 0;
        send(
            &mut state,
            ServerEvent::JobsSnapshot {
                jobs: vec![],
                server_now_ms: 0,
            },
        );
        assert_eq!(state.jobs.selected, 0);
        assert!(state.jobs.jobs.is_empty());
    }

    #[test]
    fn test_job_enqueued_dedupes() {
        let mut state = test_state();
        state.jobs.jobs.push(sample_job(1, "Queued"));
        send(
            &mut state,
            ServerEvent::JobEnqueued {
                job: sample_job(1, "Queued"),
            },
        );
        assert_eq!(state.jobs.jobs.len(), 1);
        send(
            &mut state,
            ServerEvent::JobEnqueued {
                job: sample_job(2, "Queued"),
            },
        );
        assert_eq!(state.jobs.jobs.len(), 2);
    }

    #[test]
    fn test_job_status_changed_updates_job_dto() {
        let mut state = test_state();
        state.jobs.jobs.push(sample_job(1, "Queued"));
        send(
            &mut state,
            ServerEvent::JobStatusChanged {
                id: 1,
                status: "Running".into(),
                error: None,
                started_at_ms: Some(200),
                completed_at_ms: None,
            },
        );
        assert_eq!(state.jobs.jobs[0].status, "Running");
        assert_eq!(state.jobs.jobs[0].started_at_ms, Some(200));
        assert_eq!(state.jobs.active_count, 1);
    }

    #[test]
    fn test_job_detail_changed_updates_job() {
        let mut state = test_state();
        state.jobs.jobs.push(sample_job(1, "Running"));
        let detail = vec![scylla_core::types::ChapterDetail {
            title: "1.1 Crappy Monday".into(),
            url: "u1".into(),
            status: "Done".into(),
        }];
        send(
            &mut state,
            ServerEvent::JobDetailChanged {
                id: 1,
                detail: detail.clone(),
            },
        );
        assert_eq!(state.jobs.jobs[0].detail, Some(detail));
    }

    #[test]
    fn test_chapter_to_embed_is_ignored() {
        let mut state = test_state();
        state.jobs.jobs.push(sample_job(1, "Running"));
        send(
            &mut state,
            ServerEvent::ChapterToEmbed {
                chapter: serde_json::json!({ "url": "u1", "idx": 0, "title": "Ch1" }),
            },
        );
        // No state change — the event is a no-op.
        assert_eq!(state.jobs.jobs[0].detail, None);
        assert_eq!(state.ui.page, Page::Library);
    }

    #[test]
    fn test_connection_state_sets_connected() {
        let mut state = test_state();
        assert!(!state.jobs.connected);
        send(&mut state, ServerEvent::ConnectionState { connected: true });
        assert!(state.jobs.connected);
        send(
            &mut state,
            ServerEvent::ConnectionState { connected: false },
        );
        assert!(!state.jobs.connected);
    }

    #[test]
    fn test_connection_state_disconnect_clears_reader_loading() {
        let mut state = test_state();
        state.reader.loading = true;
        send(
            &mut state,
            ServerEvent::ConnectionState { connected: false },
        );
        assert!(!state.reader.loading);
        // Reconnect must not clear loading (a fresh fetch may be in flight).
        state.reader.loading = true;
        send(&mut state, ServerEvent::ConnectionState { connected: true });
        assert!(state.reader.loading);
    }

    #[test]
    fn test_cover_fetched_decodes_into_protocol() {
        let mut state = test_state();
        // A tiny 1x1 PNG so image::load_from_memory succeeds.
        let img = image::RgbImage::new(1, 1);
        let mut bytes: Vec<u8> = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .unwrap();

        send(
            &mut state,
            ServerEvent::CoverFetched {
                url: "http://example.com/c.png".into(),
                bytes,
            },
        );
        assert!(
            state
                .lib
                .library
                .cover_cache
                .contains_key("http://example.com/c.png")
        );
    }

    #[test]
    fn test_cover_fetched_bad_bytes_logs_without_panic() {
        let mut state = test_state();
        send(
            &mut state,
            ServerEvent::CoverFetched {
                url: "http://example.com/bad.png".into(),
                bytes: vec![1, 2, 3],
            },
        );
        assert!(
            !state
                .lib
                .library
                .cover_cache
                .contains_key("http://example.com/bad.png")
        );
    }

    #[test]
    fn test_workers_changed_updates_settings() {
        let mut state = test_state();
        send(&mut state, ServerEvent::WorkersChanged { max_workers: 8 });
        assert_eq!(state.jobs.max_workers, 8);
        assert_eq!(state.lib.settings.max_workers, 8);
    }

    fn sample_hit(book_url: &str, book_title: &str, idx: usize, score: f32) -> ChapterHit {
        ChapterHit {
            book_url: book_url.into(),
            book_title: book_title.into(),
            chapter_url: format!("{}/ch{}", book_url, idx),
            chapter_idx: idx,
            chapter_title: format!("Ch{}", idx),
            score,
            genres: vec![],
        }
    }

    fn chapter_results_state(query: &str) -> AppState {
        let mut state = test_state();
        state.ui.modal = Modal::ChapterResults {
            query: query.into(),
            groups: vec![],
            cursor: 0,
            scroll_offset: 0,
            status: SearchStatus::Loading,
        };
        state
    }

    #[test]
    fn test_ai_search_results_groups_by_book() {
        let mut state = chapter_results_state("dragon");
        let hits = vec![
            sample_hit("u1", "Book A", 0, 0.5),
            sample_hit("u2", "Book B", 0, 0.9),
            sample_hit("u1", "Book A", 1, 0.8),
        ];
        send(
            &mut state,
            ServerEvent::AiSearchResults {
                query: "dragon".into(),
                result: Ok(hits),
            },
        );
        match &state.ui.modal {
            Modal::ChapterResults {
                groups,
                status,
                cursor,
                ..
            } => {
                assert_eq!(*status, SearchStatus::Ready);
                assert_eq!(*cursor, 0);
                assert_eq!(groups.len(), 2);
                // Books ordered by best chapter score desc: Book B (0.9) first.
                assert_eq!(groups[0].book_title, "Book B");
                assert_eq!(groups[1].book_title, "Book A");
                // best_score mirrors the best chapter's score.
                assert_eq!(groups[0].best_score, 0.9);
                assert_eq!(groups[1].best_score, 0.8);
                // Chapters within a group sorted by score desc.
                assert_eq!(groups[1].chapters[0].chapter_idx, 1);
                assert_eq!(groups[1].chapters[1].chapter_idx, 0);
            }
            _ => panic!("expected ChapterResults"),
        }
    }

    #[test]
    fn test_ai_search_results_empty_sets_empty_status() {
        let mut state = chapter_results_state("dragon");
        send(
            &mut state,
            ServerEvent::AiSearchResults {
                query: "dragon".into(),
                result: Ok(vec![]),
            },
        );
        match &state.ui.modal {
            Modal::ChapterResults { status, .. } => assert_eq!(*status, SearchStatus::Empty),
            _ => panic!("expected ChapterResults"),
        }
    }

    #[test]
    fn test_ai_search_results_error_sets_error_status() {
        let mut state = chapter_results_state("dragon");
        send(
            &mut state,
            ServerEvent::AiSearchResults {
                query: "dragon".into(),
                result: Err("boom".into()),
            },
        );
        match &state.ui.modal {
            Modal::ChapterResults { status, .. } => {
                assert_eq!(*status, SearchStatus::Error("boom".into()))
            }
            _ => panic!("expected ChapterResults"),
        }
    }

    #[test]
    fn test_ai_search_results_stale_query_is_dropped() {
        let mut state = chapter_results_state("dragon");
        send(
            &mut state,
            ServerEvent::AiSearchResults {
                query: "other".into(),
                result: Ok(vec![sample_hit("u1", "Book A", 0, 0.9)]),
            },
        );
        match &state.ui.modal {
            Modal::ChapterResults { status, groups, .. } => {
                assert_eq!(*status, SearchStatus::Loading);
                assert!(groups.is_empty());
            }
            _ => panic!("expected ChapterResults"),
        }
    }

    #[test]
    fn test_ai_search_results_without_modal_is_dropped() {
        let mut state = test_state();
        send(
            &mut state,
            ServerEvent::AiSearchResults {
                query: "dragon".into(),
                result: Ok(vec![sample_hit("u1", "Book A", 0, 0.9)]),
            },
        );
        assert_eq!(state.ui.modal, Modal::None);
    }

    #[test]
    fn test_build_groups_preserves_genres_and_sorts() {
        let mut hits = vec![
            sample_hit("u1", "Book A", 0, 0.5),
            sample_hit("u1", "Book A", 1, 0.9),
        ];
        hits[0].genres = vec!["Fantasy".into()];
        hits[1].genres = vec!["Fantasy".into()];
        let groups = build_groups(hits);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].genres, vec!["Fantasy"]);
        assert_eq!(groups[0].best_score, 0.9);
        assert_eq!(groups[0].chapters[0].chapter_idx, 1);
        assert_eq!(groups[0].chapters[1].chapter_idx, 0);
    }

    #[test]
    fn test_should_refresh_never_fetched() {
        let now = std::time::Instant::now();
        assert!(should_refresh(None, now));
    }

    #[test]
    fn test_should_refresh_stale_entry() {
        let now = std::time::Instant::now();
        let stale = now - std::time::Duration::from_secs(11);
        assert!(should_refresh(Some(stale), now));
    }

    #[test]
    fn test_should_refresh_recent_entry_skips() {
        let now = std::time::Instant::now();
        let recent = now - std::time::Duration::from_secs(5);
        assert!(!should_refresh(Some(recent), now));
    }
}
