//! Embed-chapters modal input — multi-select chapters to crawl for embedding.
//! Each selected chapter is enqueued as a `FetchChapter` job (the server's
//! embedding queue embeds as a side effect). Chapters that already have
//! embeddings are not selectable.

use crate::input::keybinds::*;
use crate::models::Chapter;
use crate::state::{AppState, Modal};
use crossterm::event::{KeyCode, KeyEvent};

pub fn handle_embed_chapters(state: &mut AppState, key: KeyEvent) -> bool {
    let (chapters, selected, cursor, embedded_urls) = if let Modal::EmbedChapters {
        chapters,
        selected,
        cursor,
        embedded_urls,
        ..
    } = &state.ui.modal
    {
        (
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
            if !is_embedded(&chapters[cursor], &embedded_urls)
                && let Modal::EmbedChapters { selected, .. } = &mut state.ui.modal
                && let Some(sel) = selected.get_mut(cursor)
            {
                *sel = !*sel;
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
            let base = crate::storage::client::api_base(state);
            for ch in &to_embed {
                if let Err(e) =
                    crate::storage::client::block_on(crate::storage::client::enqueue_job(
                        &base,
                        "FetchChapter",
                        &ch.url,
                        Some(ch.order as usize),
                    ))
                {
                    crate::settings::log(
                        crate::settings::LogLevel::Error,
                        "EMBED",
                        &format!("Failed to enqueue chapter {}: {}", ch.title, e),
                    );
                }
            }
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
/// direction, skipping embedded chapters. Clamps to the list bounds.
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
    idx as usize
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
    fn test_space_noop_on_embedded() {
        let mut state = embed_state();
        // Move to the embedded row (index 1) directly.
        if let Modal::EmbedChapters { cursor, .. } = &mut state.ui.modal {
            *cursor = 1;
        }
        handle_embed_chapters(&mut state, key_event(KeyCode::Char(' ')));
        assert_eq!(selected(&state), vec![false, false, false]);
    }

    #[test]
    fn test_space_toggles_selection() {
        let mut state = embed_state();
        handle_embed_chapters(&mut state, key_event(KeyCode::Char(' ')));
        assert_eq!(selected(&state), vec![true, false, false]);
        handle_embed_chapters(&mut state, key_event(KeyCode::Char(' ')));
        assert_eq!(selected(&state), vec![false, false, false]);
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
    fn test_unhandled_key_returns_true() {
        let mut state = embed_state();
        let result = handle_embed_chapters(&mut state, key_event(KeyCode::Char('x')));
        assert!(result);
    }
}
