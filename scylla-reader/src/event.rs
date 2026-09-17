use crate::models::job::JobStatus;
use crate::state::AppState;
use scylla_core::messenger::{AppCommand, AppEvent};
use std::sync::mpsc;

pub fn drain_events(state: &mut AppState, event_rx: &mpsc::Receiver<AppEvent>) {
    while let Ok(event) = event_rx.try_recv() {
        match event {
            AppEvent::BookScraped(book) => {
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
            AppEvent::ChapterFetched(chapter) => {
                crate::settings::log(
                    crate::settings::LogLevel::Debug,
                    "UI",
                    &format!("Chapter received: {}", chapter.title),
                );
                let session_id = state.reader.session_id;
                if let Some(book) = state.lib.library.selected_book_mut()
                    && let Some(session) = book.sessions.iter_mut().find(|s| s.id == session_id)
                {
                    session.progress.current = chapter.chapter_idx as u32;
                }
                if let Some(backend) = state.lib.manager.primary_backend()
                    && let Err(e) = crate::storage::client::block_on(
                        backend.update_progress(session_id, chapter.chapter_idx as u32),
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
                    chapter.title,
                    chapter.content,
                    chapter.chapter_idx,
                    state.reader.session_id,
                    state.reader.session_name.clone(),
                );
            }
            AppEvent::ChapterFetchFailed => {
                crate::settings::log(
                    crate::settings::LogLevel::Debug,
                    "UI",
                    "Chapter fetch failed",
                );
                state.reader.loading = false;
            }
            AppEvent::CoverFetched(url, protocol) => {
                crate::settings::log(
                    crate::settings::LogLevel::Debug,
                    "UI",
                    &format!("Cover fetched: {}", url),
                );
                state.lib.library.cover_cache.insert(url, protocol);
            }
            AppEvent::JobEnqueued(job) => {
                crate::settings::log(
                    crate::settings::LogLevel::Debug,
                    "UI",
                    &format!("Job enqueued: {} ({})", job.target, job.id),
                );
                state.jobs.jobs.push(job);
            }
            AppEvent::JobStatusChanged(id, status) => {
                crate::settings::log(
                    crate::settings::LogLevel::Debug,
                    "UI",
                    &format!("Job {} status: {:?}", id, status),
                );
                state.jobs.update_from_event(id, status);
                state.jobs.active_count = state
                    .jobs
                    .jobs
                    .iter()
                    .filter(|j| matches!(j.status, JobStatus::Running))
                    .count() as u8;
            }
            AppEvent::WorkersChanged(n) => {
                crate::settings::log(
                    crate::settings::LogLevel::Debug,
                    "UI",
                    &format!("Workers changed to {}", n),
                );
                state.jobs.max_workers = n;
                state.lib.settings.max_workers = n;
            }
            AppEvent::JobOutcome(id, outcome) => {
                crate::settings::log(
                    crate::settings::LogLevel::Debug,
                    "UI",
                    &format!("Job {} outcome", id),
                );
                state.jobs.set_outcome(id, outcome);
            }
            AppEvent::PluginInstalled(domain, path) => {
                crate::settings::log(
                    crate::settings::LogLevel::Debug,
                    "PLUGIN",
                    &format!("Plugin installed: {} at {}", domain, path),
                );
                state.lib.settings.reload_plugins();
            }
            AppEvent::PluginInstallFailed(msg) => {
                crate::settings::log(
                    crate::settings::LogLevel::Error,
                    "PLUGIN",
                    &format!("Plugin install failed: {}", msg),
                );
            }
        }
    }
}

pub fn update_covers(
    state: &mut AppState,
    cmd_tx: &mpsc::Sender<AppCommand>,
    fetched_covers: &mut std::collections::HashSet<String>,
) {
    let current_cover_url = state
        .lib
        .library
        .selected_book()
        .and_then(|b| b.cover_url.clone());

    if let Some(url) = current_cover_url
        && !fetched_covers.contains(&url)
    {
        fetched_covers.insert(url.clone());
        let _ = cmd_tx.send(AppCommand::FetchCover(url));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::*;
    use scylla_core::types::{BookStatus, Chapter, Progress, Session};

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

        let (tx, rx) = mpsc::channel();
        tx.send(AppEvent::BookScraped(scraped)).unwrap();
        drain_events(&mut state, &rx);

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

        let (tx, rx) = mpsc::channel();
        tx.send(AppEvent::BookScraped(sample_book("http://example.com/new")))
            .unwrap();
        drain_events(&mut state, &rx);

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

        let (tx, rx) = mpsc::channel();
        tx.send(AppEvent::BookScraped(sample_book(
            "http://example.com/book",
        )))
        .unwrap();
        drain_events(&mut state, &rx);

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

        let chapter = scylla_core::messenger::ChapterContent {
            chapter_idx: 3,
            title: "Ch3".into(),
            content: "text".into(),
        };
        let (tx, rx) = mpsc::channel();
        tx.send(AppEvent::ChapterFetched(chapter)).unwrap();
        drain_events(&mut state, &rx);

        let calls = calls.lock().unwrap().clone();
        assert!(
            calls.iter().any(|c| c == "update_progress:42:3"),
            "expected update_progress call, got: {:?}",
            calls
        );
    }
}
