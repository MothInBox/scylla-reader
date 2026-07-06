use crate::messenger::{AppCommand, AppEvent};
use crate::state::AppState;
use std::sync::mpsc;

pub fn drain_events(
    state: &mut AppState,
    event_rx: &mpsc::Receiver<AppEvent>,
    _cmd_tx: &mpsc::Sender<AppCommand>,
    last_cover_url: &mut Option<String>,
) {
    while let Ok(event) = event_rx.try_recv() {
        match event {
            AppEvent::BookScraped(book) => {
                crate::settings::log(
                    crate::settings::LogLevel::Debug,
                    "UI",
                    &format!("UI received book: {}", book.title),
                );
                if let Some(existing) = state
                    .library
                    .books
                    .iter_mut()
                    .find(|b| b.url == book.url)
                {
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
                    state.library.books.push(book);
                    if let Some(b) = state
                        .library
                        .books
                        .iter_mut()
                        .find(|b| b.url == book_url)
                    {
                        if let Ok(sessions) = state.db.load_sessions_for_book(&book_url) {
                            b.sessions = sessions;
                        }
                    }
                }
                *last_cover_url = None;
            }
            AppEvent::ChapterFetched(chapter) => {
                crate::settings::log(
                    crate::settings::LogLevel::Debug,
                    "UI",
                    &format!("Chapter received: {}", chapter.title),
                );
                if let Some(book) = state.library.selected_book_mut() {
                    if let Some(session) = book
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
                state.reader.loading = false;
            }
            AppEvent::CoverFetched(url, protocol) => {
                let current_url = state
                    .library
                    .selected_book()
                    .and_then(|b| b.cover_url.as_deref().map(str::to_owned));
                if current_url.as_deref() == Some(&url) {
                    state.library.cached_protocol = Some(protocol);
                }
            }
        }
    }
}

pub fn update_covers(
    state: &mut AppState,
    cmd_tx: &mpsc::Sender<AppCommand>,
    last_cover_url: &mut Option<String>,
) {
    let current_cover_url = state
        .library
        .selected_book()
        .and_then(|b| b.cover_url.clone());

    if current_cover_url != *last_cover_url {
        *last_cover_url = current_cover_url.clone();
        state.library.cached_protocol = None;

        if let Some(url) = current_cover_url {
            let _ = cmd_tx.send(AppCommand::FetchCover(url));
        }
    }
}
