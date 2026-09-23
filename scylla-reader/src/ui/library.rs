//! Library page renderer — book list, detail side panel, filter bar.

use crate::library::{AiRow, AiSession};
use crate::state::modal::SearchStatus;
use crate::state::{LibraryState, UiState};

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};
use ratatui_image::StatefulImage;

use crate::ui::widgets::hint_line;

pub fn draw(frame: &mut Frame, area: Rect, lib: &mut LibraryState, ui: &UiState) {
    let hint_height: u16 = if ui.show_hints { 2 } else { 1 };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(hint_height),
        ])
        .split(area);

    let filter_bar = if let Some(ai) = &lib.library.ai {
        // AI segment: query, visible result count, coverage when partial, and
        // the active genre facet (Phase 4 chip) when one is selected.
        let coverage = if ai.results.embedded < ai.results.total {
            format!(
                " ({} of {} embedded)",
                ai.results.embedded, ai.results.total
            )
        } else {
            String::new()
        };
        let genre = ai
            .genre
            .as_ref()
            .map(|g| format!(" · {}", g))
            .unwrap_or_default();
        format!(
            " Filter: {} | AI: \"{}\" ({}){}{}",
            lib.library.filter,
            ai.query,
            ai.visible_book_count(),
            genre,
            coverage
        )
    } else {
        format!(" Filter: {}", lib.library.filter)
    };
    let filter_bar = Paragraph::new(filter_bar).style(Style::default().fg(Color::Yellow));
    frame.render_widget(filter_bar, chunks[0]);

    let main_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(chunks[1]);

    draw_book_list(frame, main_chunks[0], lib);
    draw_side_panel(frame, main_chunks[1], lib);

    if ui.show_hints {
        let hint_area = chunks[2];
        let hint_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Length(1)])
            .split(hint_area);
        let actions: &[(&str, &str)] = if let Some(ai) = &lib.library.ai {
            if ai.book_mode {
                &[
                    ("Enter", "Chapters"),
                    ("g", "Chapters"),
                    (",.", "Genre"),
                    ("f", "Refine"),
                    ("Esc", "Clear AI"),
                ]
            } else {
                &[
                    ("Enter", "Open"),
                    ("g", "Books"),
                    (",.", "Genre"),
                    ("f", "Refine"),
                    ("Esc", "Clear AI"),
                ]
            }
        } else {
            &[
                ("i", "Add Book"),
                ("j", "Jump"),
                ("e", "Embed"),
                ("d", "Delete"),
                ("u", "Update"),
                ("f", "Filter"),
                ("Space", "Status"),
            ]
        };
        frame.render_widget(
            Paragraph::new(hint_line("Actions", actions)),
            hint_chunks[0],
        );
        frame.render_widget(
            Paragraph::new(hint_line("Nav", crate::ui::widgets::NAV_HINTS)),
            hint_chunks[1],
        );
    }
}

fn draw_book_list(frame: &mut Frame, area: Rect, lib: &mut LibraryState) {
    let ai_active = lib.library.ai.is_some();
    let title = if ai_active {
        " Library — AI ranked "
    } else {
        " Library "
    };
    let block = Block::default().title(title).borders(Borders::ALL);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if let Some(ai) = &lib.library.ai {
        draw_ai_list(frame, inner, ai);
        return;
    }

    let visible = lib.library.visible_indices();
    let items: Vec<ListItem> = visible
        .iter()
        .map(|&i| {
            let b = &lib.library.books[i];
            let tags = if b.tags.is_empty() {
                String::new()
            } else {
                format!(" [{}]", b.tags.join(", "))
            };
            let session_info = if b.sessions.len() > 1 {
                format!(" [{} sessions]", b.sessions.len())
            } else {
                String::new()
            };
            ListItem::new(format!(
                "{} ({}) {}/{}{}{}",
                b.title,
                b.status,
                if b.sessions.first().map(|s| s.progress.total).unwrap_or(0) == 0 {
                    0
                } else {
                    let active = b
                        .active_session_id
                        .and_then(|id| b.sessions.iter().find(|s| s.id == id))
                        .or_else(|| b.sessions.first());
                    match active {
                        Some(s) => (s.progress.current + 1).min(s.progress.total),
                        None => 0,
                    }
                },
                b.sessions.first().map(|s| s.progress.total).unwrap_or(0),
                tags,
                session_info,
            ))
        })
        .collect();

    let mut list_state = ListState::default();
    list_state.select(Some(lib.library.selected_index));

    let list = List::new(items)
        .highlight_style(Style::default().bg(Color::Blue).fg(Color::White))
        .highlight_symbol(">> ");

    frame.render_stateful_widget(list, inner, &mut list_state);
}

/// Render the AI-ranked list (or its status message) inside the list area.
fn draw_ai_list(frame: &mut Frame, area: Rect, ai: &AiSession) {
    match &ai.status {
        SearchStatus::Loading => {
            frame.render_widget(
                Paragraph::new("searching…").alignment(Alignment::Center),
                area,
            );
        }
        SearchStatus::NoEmbeddings { total } => {
            let para = Paragraph::new(format!(
                "0 of {} chapters embedded yet — press e on a book in the library,\nor enable auto-embed in Settings.",
                total
            ))
            .wrap(Wrap { trim: true })
            .alignment(Alignment::Center);
            frame.render_widget(para, area);
        }
        SearchStatus::Empty => {
            frame.render_widget(
                Paragraph::new("No matches — try rephrasing or fewer details.")
                    .alignment(Alignment::Center),
                area,
            );
        }
        SearchStatus::NoChapters => {
            frame.render_widget(
                Paragraph::new("No matching chapters in this book.").alignment(Alignment::Center),
                area,
            );
        }
        SearchStatus::Error(msg) => {
            frame.render_widget(
                Paragraph::new(format!("Search failed: {}", msg)).alignment(Alignment::Center),
                area,
            );
        }
        SearchStatus::Ready => {
            let rows = ai.rows();
            let items: Vec<ListItem> = rows
                .iter()
                .map(|row| match row {
                    AiRow::Book { book_index } => {
                        let b = &ai.results.books[*book_index];
                        ListItem::new(Line::from(vec![
                            Span::styled(
                                format!(" {}", b.book_title),
                                Style::default()
                                    .fg(Color::Cyan)
                                    .add_modifier(Modifier::BOLD),
                            ),
                            Span::styled(
                                format!("{:>6.0}", b.score),
                                Style::default().fg(Color::DarkGray),
                            ),
                        ]))
                    }
                    AiRow::Chapter {
                        book_index,
                        chapter_index,
                    } => {
                        let c = &ai.results.books[*book_index].chapters[*chapter_index];
                        let score = format!("{:>6.0}", c.score);
                        let mut lines = vec![Line::from(vec![
                            Span::raw(format!("   {}", c.chapter_title)),
                            Span::styled(score, Style::default().fg(Color::DarkGray)),
                        ])];
                        // Phase 4 snippet: a dimmed line under the title when a
                        // matching span is present (old servers → no line).
                        if !c.snippet.trim().is_empty() {
                            lines.push(Line::from(Span::styled(
                                truncate_snippet(
                                    c.snippet.trim(),
                                    area.width.saturating_sub(6) as usize,
                                ),
                                Style::default().fg(Color::DarkGray),
                            )));
                        }
                        ListItem::new(lines)
                    }
                })
                .collect();
            let mut list_state = ListState::default();
            list_state.select(Some(ai.cursor));
            let list = List::new(items)
                .highlight_style(Style::default().bg(Color::Blue).fg(Color::White))
                .highlight_symbol(">> ");
            frame.render_stateful_widget(list, area, &mut list_state);
        }
    }
}

fn draw_side_panel(frame: &mut Frame, area: Rect, lib: &mut LibraryState) {
    let block = Block::default().title(" Details ").borders(Borders::ALL);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let book_data = lib.library.selected_book().map(|b| {
        (
            b.url.clone(),
            b.title.clone(),
            b.status.clone(),
            b.sessions.clone(),
            b.active_session_id,
            b.tags.clone(),
            b.description.clone(),
            b.cover_url.clone(),
        )
    });

    let Some((book_url, title, status, sessions, active_session_id, tags, description, cover_url)) =
        book_data
    else {
        frame.render_widget(Paragraph::new("No book selected"), inner);
        return;
    };

    let side_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(12), Constraint::Min(0)])
        .split(inner);

    if let Some(url) = &cover_url
        && let Some(protocol) = lib.library.cover_cache.get_mut(url)
    {
        frame.render_stateful_widget(StatefulImage::new(None), side_chunks[0], protocol);
    }

    let active_session = active_session_id
        .and_then(|id| sessions.iter().find(|s| s.id == id))
        .or_else(|| sessions.first());

    let progress_str = match active_session {
        Some(s) => format!(
            "{}/{} chapters [Session: {}]",
            if s.progress.total == 0 {
                0
            } else {
                (s.progress.current + 1).min(s.progress.total)
            },
            s.progress.total,
            s.name,
        ),
        None => "No sessions".to_string(),
    };

    let session_count = format!("Sessions:  {}", sessions.len());

    // Embedding progress for the selected book, when known.
    let embedding_line = lib
        .library
        .embedding_status_cache
        .get(&book_url)
        .map(|s| {
            let mut lines = vec![format!(
                "Embedded: {}/{}",
                s.embedded_chapters, s.total_chapters
            )];
            if !s.genres.is_empty() {
                lines.push(format!("Genres:   {}", s.genres.join(", ")));
            }
            lines.join("\n")
        })
        .unwrap_or_default();

    let mut details = format!(
        "Title:    {}\nStatus:   {}\nProgress: {}\n{}\nTags:     {}",
        title,
        status,
        progress_str,
        session_count,
        tags.join(", "),
    );
    if !embedding_line.is_empty() {
        details.push_str(&format!("\n{}", embedding_line));
    }
    details.push_str(&format!("\n\n{}", description.unwrap_or_default()));
    frame.render_widget(
        Paragraph::new(details).wrap(Wrap { trim: false }),
        side_chunks[1],
    );
}

/// Truncate a snippet to `width` visible characters, appending an ellipsis when
/// it was cut. Multi-byte safe (counts chars, not bytes).
fn truncate_snippet(s: &str, width: usize) -> String {
    if width == 0 || s.chars().count() <= width {
        return s.to_string();
    }
    let mut out: String = s.chars().take(width.saturating_sub(1)).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::Library;
    use crate::state::AppState;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn make_state() -> AppState {
        AppState::from_parts(Library::new())
    }

    #[test]
    fn test_library_draw_shows_hints_when_enabled() {
        let mut state = make_state();
        state.ui.show_hints = true;

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                draw(f, f.area(), &mut state.lib, &state.ui);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        let content: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(content.contains("[1]"));
    }

    #[test]
    fn test_library_draw_shows_book_titles() {
        let mut state = make_state();
        state
            .lib
            .library
            .add_book("Test Book Title".into(), "url".into());

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                draw(f, f.area(), &mut state.lib, &state.ui);
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        let content: String = buffer.content().iter().map(|c| c.symbol()).collect();
        assert!(content.contains("Test Book Title"));
    }

    #[test]
    fn test_library_draw_hides_hints_when_disabled() {
        let mut state = make_state();
        state.ui.show_hints = false;

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                draw(f, f.area(), &mut state.lib, &state.ui);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        let content: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(!content.contains("[1]"));
    }

    #[test]
    fn test_library_draw_shows_embedding_status() {
        let mut state = make_state();
        state
            .lib
            .library
            .add_book("Test Book".into(), "http://example.com/book".into());
        state.lib.library.embedding_status_cache.insert(
            "http://example.com/book".into(),
            crate::storage::client::EmbeddingStatus {
                embedded_chapters: 12,
                total_chapters: 40,
                aggregate: true,
                genres: vec!["Fantasy".into(), "LitRPG".into()],
                embedded_chapter_urls: vec![],
            },
        );

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                draw(f, f.area(), &mut state.lib, &state.ui);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        let content: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(content.contains("Embedded: 12/40"), "content: {}", content);
        assert!(
            content.contains("Genres:   Fantasy, LitRPG"),
            "content: {}",
            content
        );
    }

    #[test]
    fn test_library_draw_embedding_status_without_genres() {
        let mut state = make_state();
        state
            .lib
            .library
            .add_book("Test Book".into(), "http://example.com/book".into());
        state.lib.library.embedding_status_cache.insert(
            "http://example.com/book".into(),
            crate::storage::client::EmbeddingStatus {
                embedded_chapters: 0,
                total_chapters: 5,
                aggregate: false,
                genres: vec![],
                embedded_chapter_urls: vec![],
            },
        );

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                draw(f, f.area(), &mut state.lib, &state.ui);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        let content: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(content.contains("Embedded: 0/5"), "content: {}", content);
        assert!(!content.contains("Genres:"), "content: {}", content);
    }

    // ── AI indicator rendering ──────────────────────────────────────────────

    fn ai_state() -> AppState {
        let mut state = make_state();
        state.lib.library.add_book("Book A".into(), "u1".into());
        state.lib.library.add_book("Book B".into(), "u2".into());
        state.lib.library.apply_ai_results(
            "dragon heart".into(),
            crate::event_types::SearchOutcome::BookMode {
                embedded: 12,
                total: 15,
                hits: vec![
                    crate::event_types::AiBook {
                        book_url: "u2".into(),
                        book_title: "Book B".into(),
                        score: 90.0,
                        genres: vec![],
                        chapters: vec![crate::event_types::AiChapter {
                            chapter_url: "u2/ch0".into(),
                            chapter_idx: 0,
                            chapter_title: "Scene 1".into(),
                            score: 90.0,
                            snippet: String::new(),
                        }],
                    },
                    crate::event_types::AiBook {
                        book_url: "u1".into(),
                        book_title: "Book A".into(),
                        score: 70.0,
                        genres: vec![],
                        chapters: vec![],
                    },
                ],
            },
        );
        state
    }

    fn draw_lib(state: &mut AppState) -> String {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                draw(f, f.area(), &mut state.lib, &state.ui);
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        buf.content().iter().map(|c| c.symbol()).collect()
    }

    #[test]
    fn test_library_draw_shows_ai_filter_bar_and_title() {
        let mut state = ai_state();
        let content = draw_lib(&mut state);
        assert!(
            content.contains("AI: \"dragon heart\" (2)"),
            "content: {}",
            content
        );
        assert!(
            content.contains("Library — AI ranked"),
            "content: {}",
            content
        );
    }

    #[test]
    fn test_library_draw_shows_ai_coverage_when_partial() {
        let mut state = ai_state();
        let content = draw_lib(&mut state);
        assert!(
            content.contains("(12 of 15 embedded)"),
            "content: {}",
            content
        );
    }

    #[test]
    fn test_library_draw_shows_ai_footer_hints() {
        let mut state = ai_state();
        let content = draw_lib(&mut state);
        assert!(content.contains("Enter"), "content: {}", content);
        assert!(content.contains("Clear AI"), "content: {}", content);
        assert!(content.contains("Refine"), "content: {}", content);
    }

    #[test]
    fn test_library_draw_shows_ai_rows_with_scores() {
        let mut state = ai_state();
        let content = draw_lib(&mut state);
        assert!(content.contains("Book B"), "content: {}", content);
        assert!(content.contains("90"), "content: {}", content);
    }

    #[test]
    fn test_library_draw_grouped_mode_shows_chapters() {
        let mut state = ai_state();
        state.lib.library.toggle_ai_mode(); // grouped-chapter
        let content = draw_lib(&mut state);
        assert!(content.contains("Scene 1"), "content: {}", content);
        assert!(content.contains("Book B"), "content: {}", content);
    }

    #[test]
    fn test_library_draw_no_embeddings_hint() {
        let mut state = make_state();
        state.lib.library.apply_ai_results(
            "q".into(),
            crate::event_types::SearchOutcome::NoEmbeddings { total: 12 },
        );
        let content = draw_lib(&mut state);
        assert!(
            content.contains("0 of 12 chapters embedded yet"),
            "content: {}",
            content
        );
        assert!(
            content.contains("enable auto-embed in Settings"),
            "content: {}",
            content
        );
    }

    /// A state whose AI book-mode results carry genres across two books.
    fn genre_state() -> AppState {
        let mut state = make_state();
        state.lib.library.add_book("Book A".into(), "u1".into());
        state.lib.library.add_book("Book B".into(), "u2".into());
        state.lib.library.apply_ai_results(
            "dragon heart".into(),
            crate::event_types::SearchOutcome::BookMode {
                embedded: 12,
                total: 15,
                hits: vec![
                    crate::event_types::AiBook {
                        book_url: "u2".into(),
                        book_title: "Book B".into(),
                        score: 90.0,
                        genres: vec!["Fantasy".into()],
                        chapters: vec![],
                    },
                    crate::event_types::AiBook {
                        book_url: "u1".into(),
                        book_title: "Book A".into(),
                        score: 70.0,
                        genres: vec!["Sci-Fi".into()],
                        chapters: vec![],
                    },
                ],
            },
        );
        state
    }

    #[test]
    fn test_library_draw_shows_genre_chip_when_active() {
        let mut state = genre_state();
        state.lib.library.cycle_ai_genre(1); // Fantasy
        let content = draw_lib(&mut state);
        assert!(
            content.contains("AI: \"dragon heart\" (1) · Fantasy"),
            "content: {}",
            content
        );
    }

    #[test]
    fn test_library_draw_no_genre_chip_when_clear() {
        let mut state = genre_state();
        let content = draw_lib(&mut state);
        assert!(
            content.contains("AI: \"dragon heart\" (2)"),
            "content: {}",
            content
        );
        assert!(!content.contains(" · Fantasy"), "content: {}", content);
    }

    #[test]
    fn test_library_draw_genre_filter_narrows_result_count() {
        let mut state = genre_state();
        state.lib.library.cycle_ai_genre(1); // Fantasy → only Book B
        let content = draw_lib(&mut state);
        // The count reflects the genre-filtered set, and Book A is hidden.
        assert!(
            content.contains("AI: \"dragon heart\" (1)"),
            "content: {}",
            content
        );
        assert!(content.contains("Book B"), "content: {}", content);
        assert!(!content.contains("Book A"), "content: {}", content);
    }

    #[test]
    fn test_library_draw_shows_genre_footer_hint() {
        let mut state = genre_state();
        let content = draw_lib(&mut state);
        assert!(content.contains("[,.]"), "content: {}", content);
        assert!(content.contains("Genre"), "content: {}", content);
    }

    #[test]
    fn test_library_draw_grouped_mode_shows_snippet() {
        let mut state = ai_state();
        if let Some(ai) = &mut state.lib.library.ai {
            ai.results.books[0].chapters[0].snippet = "the dragon circled the spire".into();
        }
        state.lib.library.toggle_ai_mode(); // grouped-chapter
        let content = draw_lib(&mut state);
        assert!(
            content.contains("the dragon circled the spire"),
            "content: {}",
            content
        );
    }

    #[test]
    fn test_library_draw_grouped_mode_omits_empty_snippet_line() {
        let mut state = ai_state(); // snippet is empty
        state.lib.library.toggle_ai_mode(); // grouped-chapter
        let content = draw_lib(&mut state);
        assert!(content.contains("Scene 1"), "content: {}", content);
    }

    #[test]
    fn test_truncate_snippet_cuts_and_appends_ellipsis() {
        let long = "x".repeat(200);
        let out = truncate_snippet(&long, 20);
        assert_eq!(out.chars().count(), 20);
        assert!(out.ends_with('…'));
        // Short snippets are kept whole.
        assert_eq!(truncate_snippet("short", 20), "short");
        assert_eq!(truncate_snippet("", 20), "");
    }
}
