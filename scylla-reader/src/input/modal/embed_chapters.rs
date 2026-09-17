//! Embed-chapters modal input — multi-select chapters to crawl for embedding.
//! Each selected chapter is enqueued as a `FetchChapter` job (the server's
//! embedding queue embeds as a side effect). Chapters that already have
//! embeddings are not selectable.

use crate::input::keybinds::*;
use crate::models::Chapter;
use crate::state::{AppState, Modal};
use crossterm::event::{KeyCode, KeyEvent};

pub fn handle_embed_chapters(state: &mut AppState, key: KeyEvent) -> bool {
    let (book_url, chapters, selected, cursor, embedded_urls) = if let Modal::EmbedChapters {
        book_url,
        chapters,
        selected,
        cursor,
        embedded_urls,
        ..
    } = &state.ui.modal
    {
        (
            book_url.clone(),
            chapters.clone(),
            selected.clone(),
            *cursor,
            embedded_urls.clone(),
        )
    } else {
        return true;
    };

    match key.code {
        KEY_NAV_UP => {
            let next = next_selectable(cursor, &chapters, &embedded_urls, -1);
            if next != cursor
                && let Modal::EmbedChapters { cursor: c, .. } = &mut state.ui.modal
            {
                *c = next;
            }
            true
        }
        KEY_NAV_DOWN => {
            let next = next_selectable(cursor, &chapters, &embedded_urls, 1);
            if next != cursor
                && let Modal::EmbedChapters { cursor: c, .. } = &mut state.ui.modal
            {
                *c = next;
            }
            true
        }
        KeyCode::Char(' ') => {
            if is_embedded(&chapters[cursor], &embedded_urls) {
                // Space on an embedded row (shouldn't normally happen — the
                // cursor never rests there) — skip forward to the next
                // selectable chapter (no wrap, no toggle).
                let next = next_selectable(cursor, &chapters, &embedded_urls, 1);
                if next != cursor
                    && let Modal::EmbedChapters { cursor: c, .. } = &mut state.ui.modal
                {
                    *c = next;
                }
            } else if let Modal::EmbedChapters {
                selected,
                cursor: c,
                ..
            } = &mut state.ui.modal
                && let Some(sel) = selected.get_mut(cursor)
            {
                // Toggle the current chapter, then auto-advance to the next
                // selectable row (stay put if none ahead).
                *sel = !*sel;
                let next = next_selectable(cursor, &chapters, &embedded_urls, 1);
                if next != cursor {
                    *c = next;
                }
            }
            true
        }
        KeyCode::Char('a') => {
            if let Modal::EmbedChapters { selected, .. } = &mut state.ui.modal {
                for (i, sel) in selected.iter_mut().enumerate() {
                    if !is_embedded(&chapters[i], &embedded_urls) {
                        *sel = true;
                    }
                }
            }
            true
        }
        KeyCode::Char('c') => {
            if let Modal::EmbedChapters { selected, .. } = &mut state.ui.modal {
                for sel in selected.iter_mut() {
                    *sel = false;
                }
            }
            true
        }
        KEY_ENTER => {
            let to_embed = selected_embeddable_chapters(&chapters, &selected, &embedded_urls);
            if to_embed.is_empty() {
                // Nothing to embed — no-op, stay open.
                return true;
            }
            // Enqueue ONE EmbedBatch job carrying all selected chapters; the
            // server scrapes + embeds each and reports per-chapter detail.
            let batch: Vec<(String, usize, String)> = to_embed
                .iter()
                .map(|ch| (ch.url.clone(), ch.order as usize, ch.title.clone()))
                .collect();
            let base = crate::storage::client::api_base(state);
            if let Err(e) = crate::storage::client::block_on(
                crate::storage::client::enqueue_embed_batch(&base, &batch),
            ) {
                crate::settings::log(
                    crate::settings::LogLevel::Error,
                    "EMBED",
                    &format!("Failed to enqueue embed batch: {}", e),
                );
            }
            // Invalidate the cached embedding status so the next refresh tick
            // re-fetches and shows the newly enqueued chapters as in-progress.
            state.lib.library.embedding_status_cache.remove(&book_url);
            state
                .lib
                .library
                .embedding_status_fetched_at
                .remove(&book_url);
            state.ui.modal = Modal::None;
            true
        }
        _ => true,
    }
}

/// Whether a chapter already has an embedding (its URL is in `embedded_urls`).
fn is_embedded(ch: &Chapter, embedded_urls: &[String]) -> bool {
    embedded_urls.iter().any(|u| u == &ch.url)
}

/// Move the cursor to the next selectable (non-embedded) row in `dir`
/// direction, skipping embedded chapters. Returns `cursor` unchanged when no
/// selectable row exists in that direction, so the cursor never rests on a
/// dimmed (embedded) row.
fn next_selectable(
    cursor: usize,
    chapters: &[Chapter],
    embedded_urls: &[String],
    dir: i32,
) -> usize {
    let len = chapters.len();
    if len == 0 {
        return 0;
    }
    let mut idx = (cursor as i32 + dir).clamp(0, len as i32 - 1);
    while is_embedded(&chapters[idx as usize], embedded_urls) {
        let next = idx + dir;
        if next < 0 || next >= len as i32 {
            break;
        }
        idx = next;
    }
    // If we ended on an embedded row, there is no selectable row in this
    // direction — stay put rather than resting on a dimmed row.
    if is_embedded(&chapters[idx as usize], embedded_urls) {
        cursor
    } else {
        idx as usize
    }
}

/// The index of the first selectable (non-embedded) chapter, or 0 if all
/// chapters are embedded.
pub fn first_selectable(chapters: &[Chapter], embedded_urls: &[String]) -> usize {
    chapters
        .iter()
        .position(|ch| !is_embedded(ch, embedded_urls))
        .unwrap_or(0)
}

/// The chapters currently selected for embedding.
fn selected_chapters(chapters: &[Chapter], selected: &[bool]) -> Vec<Chapter> {
    chapters
        .iter()
        .zip(selected.iter())
        .filter(|(_, sel)| **sel)
        .map(|(ch, _)| ch.clone())
        .collect()
}

/// The selected chapters that still need embedding (not already embedded).
fn selected_embeddable_chapters(
    chapters: &[Chapter],
    selected: &[bool],
    embedded_urls: &[String],
) -> Vec<Chapter> {
    selected_chapters(chapters, selected)
        .into_iter()
        .filter(|ch| !is_embedded(ch, embedded_urls))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::*;
    use crossterm::event::KeyCode;

    fn chapter(order: u32, title: &str) -> Chapter {
        Chapter {
            title: title.into(),
            url: format!("u{}", order),
            order,
        }
    }

    fn embed_state() -> AppState {
        let mut state = test_state();
        state.ui.modal = Modal::EmbedChapters {
            book_url: "http://example.com/book".into(),
            chapters: vec![chapter(0, "Ch0"), chapter(1, "Ch1"), chapter(2, "Ch2")],
            selected: vec![false, false, false],
            embedded_urls: vec!["u1".into()],
            cursor: 0,
            scroll_offset: 0,
        };
        state
    }

    fn selected(state: &AppState) -> Vec<bool> {
        if let Modal::EmbedChapters { selected, .. } = &state.ui.modal {
            selected.clone()
        } else {
            panic!("Expected EmbedChapters modal");
        }
    }

    fn cursor(state: &AppState) -> usize {
        if let Modal::EmbedChapters { cursor, .. } = &state.ui.modal {
            *cursor
        } else {
            panic!("Expected EmbedChapters modal");
        }
    }

    #[test]
    fn test_selected_chapters_pure_fn() {
        let chapters = vec![chapter(0, "A"), chapter(1, "B"), chapter(2, "C")];
        let selected = vec![true, false, true];
        let picked = selected_chapters(&chapters, &selected);
        assert_eq!(picked.len(), 2);
        assert_eq!(picked[0].title, "A");
        assert_eq!(picked[1].title, "C");
        assert!(selected_chapters(&chapters, &vec![false, false, false]).is_empty());
    }

    #[test]
    fn test_next_selectable_pure_fn() {
        let chapters = vec![chapter(0, "A"), chapter(1, "B"), chapter(2, "C")];
        let embedded = vec!["u1".to_string()];
        // Down from 0 skips the embedded row 1 → lands on 2.
        assert_eq!(next_selectable(0, &chapters, &embedded, 1), 2);
        // Up from 2 skips the embedded row 1 → lands on 0.
        assert_eq!(next_selectable(2, &chapters, &embedded, -1), 0);
        // Clamps at the ends.
        assert_eq!(next_selectable(2, &chapters, &embedded, 1), 2);
        assert_eq!(next_selectable(0, &chapters, &embedded, -1), 0);
        // No embedded rows: normal movement.
        assert_eq!(next_selectable(0, &chapters, &[], 1), 1);
        assert_eq!(next_selectable(1, &chapters, &[], -1), 0);
    }

    #[test]
    fn test_next_selectable_never_returns_embedded() {
        // Last selectable row (0) followed only by embedded rows (1): down must
        // stay put rather than rest on the dimmed row.
        let chapters = vec![chapter(0, "A"), chapter(1, "B")];
        let embedded = vec!["u1".to_string()];
        assert_eq!(next_selectable(0, &chapters, &embedded, 1), 0);
        // All rows embedded: any direction stays put.
        let all_embedded = vec!["u0".to_string(), "u1".to_string()];
        assert_eq!(next_selectable(0, &chapters, &all_embedded, 1), 0);
        assert_eq!(next_selectable(1, &chapters, &all_embedded, -1), 1);
    }

    #[test]
    fn test_first_selectable_pure_fn() {
        let chapters = vec![chapter(0, "A"), chapter(1, "B"), chapter(2, "C")];
        // First non-embedded is 0.
        assert_eq!(first_selectable(&chapters, &["u1".to_string()]), 0);
        // ch0 and ch1 embedded → first non-embedded is 2.
        assert_eq!(
            first_selectable(&chapters, &["u0".to_string(), "u1".to_string()]),
            2
        );
        // All embedded → 0 (everything dimmed).
        assert_eq!(
            first_selectable(
                &chapters,
                &["u0".to_string(), "u1".to_string(), "u2".to_string()]
            ),
            0
        );
    }

    #[test]
    fn test_nav_skips_embedded_rows() {
        let mut state = embed_state();
        // u1 (index 1) is embedded — down from 0 lands on 2.
        handle_embed_chapters(&mut state, key_event(KEY_NAV_DOWN));
        assert_eq!(cursor(&state), 2);
        // Up from 2 skips the embedded row → lands on 0.
        handle_embed_chapters(&mut state, key_event(KEY_NAV_UP));
        assert_eq!(cursor(&state), 0);
    }

    #[test]
    fn test_nav_never_rests_on_embedded() {
        // Book with only one selectable row (0) followed by an embedded row (1):
        // down must stay put, never resting on the dimmed row.
        let mut state = test_state();
        state.ui.modal = Modal::EmbedChapters {
            book_url: "http://example.com/book".into(),
            chapters: vec![chapter(0, "Ch0"), chapter(1, "Ch1")],
            selected: vec![false, false],
            embedded_urls: vec!["u1".into()],
            cursor: 0,
            scroll_offset: 0,
        };
        handle_embed_chapters(&mut state, key_event(KEY_NAV_DOWN));
        assert_eq!(cursor(&state), 0);
    }

    #[test]
    fn test_space_on_embedded_skips_to_next_selectable() {
        let mut state = embed_state();
        // Move to the embedded row (index 1) directly.
        if let Modal::EmbedChapters { cursor, .. } = &mut state.ui.modal {
            *cursor = 1;
        }
        handle_embed_chapters(&mut state, key_event(KeyCode::Char(' ')));
        // Cursor skips forward to the next non-embedded row (index 2)…
        assert_eq!(cursor(&state), 2);
        // …and nothing is toggled.
        assert_eq!(selected(&state), vec![false, false, false]);
    }

    #[test]
    fn test_space_on_embedded_with_no_next_selectable_stays_put() {
        let mut state = embed_state();
        // Make the last row embedded too, so index 1 has no selectable row ahead.
        if let Modal::EmbedChapters {
            cursor,
            embedded_urls,
            ..
        } = &mut state.ui.modal
        {
            *cursor = 1;
            embedded_urls.push("u2".into());
        }
        handle_embed_chapters(&mut state, key_event(KeyCode::Char(' ')));
        assert_eq!(cursor(&state), 1);
        assert_eq!(selected(&state), vec![false, false, false]);
    }

    #[test]
    fn test_space_toggles_and_advances() {
        let mut state = embed_state();
        // Space on row 0 toggles it AND advances to the next selectable row.
        handle_embed_chapters(&mut state, key_event(KeyCode::Char(' ')));
        assert_eq!(selected(&state), vec![true, false, false]);
        assert_eq!(cursor(&state), 2); // skips embedded row 1
        // Space again toggles row 2 and stays put (no selectable row ahead).
        handle_embed_chapters(&mut state, key_event(KeyCode::Char(' ')));
        assert_eq!(selected(&state), vec![true, false, true]);
        assert_eq!(cursor(&state), 2);
    }

    #[test]
    fn test_select_all_skips_embedded() {
        let mut state = embed_state();
        handle_embed_chapters(&mut state, key_event(KeyCode::Char('a')));
        assert_eq!(selected(&state), vec![true, false, true]);
        handle_embed_chapters(&mut state, key_event(KeyCode::Char('c')));
        assert_eq!(selected(&state), vec![false, false, false]);
    }

    #[test]
    fn test_selected_embeddable_chapters_pure_fn() {
        let chapters = vec![chapter(0, "A"), chapter(1, "B"), chapter(2, "C")];
        let selected = vec![true, true, true];
        let embedded = vec!["u1".to_string()];
        let to_embed = selected_embeddable_chapters(&chapters, &selected, &embedded);
        assert_eq!(to_embed.len(), 2);
        assert_eq!(to_embed[0].title, "A");
        assert_eq!(to_embed[1].title, "C");
    }

    #[test]
    fn test_enter_with_no_selection_stays_open() {
        let mut state = embed_state();
        handle_embed_chapters(&mut state, key_event(KEY_ENTER));
        assert!(matches!(state.ui.modal, Modal::EmbedChapters { .. }));
    }

    #[test]
    fn test_enter_with_selection_closes_modal() {
        let mut state = embed_state();
        handle_embed_chapters(&mut state, key_event(KeyCode::Char('a')));
        handle_embed_chapters(&mut state, key_event(KEY_ENTER));
        assert_eq!(state.ui.modal, Modal::None);
    }

    #[test]
    fn test_enter_invalidates_embedding_status_cache() {
        let mut state = embed_state();
        // Seed a cached status for the modal's book.
        state.lib.library.embedding_status_cache.insert(
            "http://example.com/book".into(),
            crate::storage::client::EmbeddingStatus {
                embedded_chapters: 0,
                total_chapters: 3,
                aggregate: false,
                genres: vec![],
                embedded_chapter_urls: vec![],
            },
        );
        state
            .lib
            .library
            .embedding_status_fetched_at
            .insert("http://example.com/book".into(), std::time::Instant::now());
        handle_embed_chapters(&mut state, key_event(KeyCode::Char('a')));
        handle_embed_chapters(&mut state, key_event(KEY_ENTER));
        // The cache + fetch-time entries are removed so the next tick re-fetches.
        assert!(
            !state
                .lib
                .library
                .embedding_status_cache
                .contains_key("http://example.com/book")
        );
        assert!(
            !state
                .lib
                .library
                .embedding_status_fetched_at
                .contains_key("http://example.com/book")
        );
    }

    #[test]
    fn test_unhandled_key_returns_true() {
        let mut state = embed_state();
        let result = handle_embed_chapters(&mut state, key_event(KeyCode::Char('x')));
        assert!(result);
    }
}
