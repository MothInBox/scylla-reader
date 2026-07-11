use crate::messenger::{AppCommand, AppEvent};
use crate::models::job::JobStatus;
use crate::state::AppState;
use std::sync::mpsc;

pub fn drain_events(
    state: &mut AppState,
    event_rx: &mpsc::Receiver<AppEvent>,
) {
    while let Ok(event) = event_rx.try_recv() {
        match event {
            AppEvent::BookScraped(book) => {
                crate::settings::log(
                    crate::settings::LogLevel::Debug,
                    "UI",
                    &format!("UI received book: {}", book.title),
                );
                if let Some(existing) = state.lib.library.books.iter_mut().find(|b| b.url == book.url) {
                    existing.title = book.title.clone();
                    existing.cover_url = book.cover_url.clone();
                    existing.description = book.description.clone();
                    existing.chapters = book.chapters.clone();
                    state.db.upsert_book(existing).unwrap_or_else(|e| {
                        crate::settings::log(
                            crate::settings::LogLevel::Debug,
                            "UI",
                            &format!("DB upsert failed: {}", e),
                        );
                    });
                    if let Ok(sessions) = state.db.load_sessions_for_book(&existing.url) {
                        existing.sessions = sessions;
                    }
                } else {
                    state.db.upsert_book(&book).unwrap_or_else(|e| {
                        crate::settings::log(
                            crate::settings::LogLevel::Debug,
                            "UI",
                            &format!("DB upsert failed: {}", e),
                        );
                    });
                    let book_url = book.url.clone();
                    state.lib.library.books.push(book);
                    if let Some(b) = state.lib.library.books.iter_mut().find(|b| b.url == book_url)
                        && let Ok(sessions) = state.db.load_sessions_for_book(&book_url)
                    {
                        b.sessions = sessions;
                    }
                }
            }
            AppEvent::ChapterFetched(chapter) => {
                crate::settings::log(
                    crate::settings::LogLevel::Debug,
                    "UI",
                    &format!("Chapter received: {}", chapter.title),
                );
                if let Some(book) = state.lib.library.selected_book_mut()
                    && let Some(session) = book
                        .sessions
                        .iter_mut()
                        .find(|s| s.id == state.reader.session_id)
                {
                    session.progress.current = chapter.chapter_idx as u32;
                    state
                        .db
                        .update_session_progress(session.id, session.progress.current)
                        .unwrap_or_else(|e| {
                            crate::settings::log(
                                crate::settings::LogLevel::Debug,
                                "UI",
                                &format!("DB session progress update failed: {}", e),
                            );
                        });
                    state
                        .db
                        .set_active_session(&book.url, Some(session.id))
                        .unwrap_or_else(|e| {
                            crate::settings::log(
                                crate::settings::LogLevel::Debug,
                                "UI",
                                &format!("DB set active session failed: {}", e),
                            );
                        });
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
        .lib.library
        .selected_book()
        .and_then(|b| b.cover_url.clone());

    if let Some(url) = current_cover_url
        && !fetched_covers.contains(&url)
    {
        fetched_covers.insert(url.clone());
        let _ = cmd_tx.send(AppCommand::FetchCover(url));
    }
}
