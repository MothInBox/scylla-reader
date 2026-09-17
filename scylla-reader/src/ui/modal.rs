//! Modal popup renderer — add-book form and jump-to-chapter list.

use crate::event_types::ChapterGroup;
use crate::library::{BookFilter, filter_tags, known_tags};
use crate::models::{BookStatus, Session};
use crate::state::modal::{FilterRow, Modal, SearchStatus};
use crate::state::{LibraryState, UiState};
use crate::ui::palette::draw_palette;
use crate::ui::widgets::centered_rect;
use ratatui::prelude::*;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph};

pub fn draw_modal(frame: &mut Frame, area: Rect, ui: &mut UiState, lib: &mut LibraryState) {
    if matches!(ui.modal, Modal::CommandPalette { .. }) {
        draw_palette(frame, area, ui);
        return;
    }

    match &mut ui.modal {
        Modal::None => {}
        Modal::AddBook {
            inputs,
            cursor,
            scroll_offset,
        } => {
            let popup_area = centered_rect(70, 50, area);
            frame.render_widget(Clear, popup_area);
            let count = inputs.len();
            let items: Vec<ListItem> = inputs
                .iter()
                .map(|l| ListItem::new(l.to_string()))
                .collect();

            let title = format!(
                " Add Books ({} URL{}) ",
                count,
                if count == 1 { "" } else { "s" }
            );

            draw_scrollable_list(
                frame,
                popup_area,
                title,
                items,
                *cursor,
                scroll_offset,
                " [Enter] New line  [Ctrl+s] Submit all  [↑↓] Move  [Backspace] Delete  [Esc] Cancel",
            );
        }

        Modal::JumpChapter {
            chapters: _,
            query,
            filtered,
            cursor,
            scroll_offset,
            show_titles,
        } => {
            let popup_area = centered_rect(70, 50, area);
            frame.render_widget(Clear, popup_area);
            let count = filtered.len();
            let items: Vec<ListItem> = filtered
                .iter()
                .map(|ch| {
                    ListItem::new(if *show_titles {
                        ch.title.clone()
                    } else {
                        ch.url.clone()
                    })
                })
                .collect();
            let title = format!(" Jump to Chapter ({}) > {} ", count, query);
            draw_scrollable_list(
                frame,
                popup_area,
                title,
                items,
                *cursor,
                scroll_offset,
                " [Enter] Jump  [↑↓] Move  [t] Link/Title  [Esc] Cancel",
            );
        }

        Modal::SessionPicker {
            book_url,
            cursor,
            scroll_offset,
            input,
            editing_id,
            pending_delete_url,
        } => {
            let popup_area = centered_rect(60, 50, area);
            frame.render_widget(Clear, popup_area);

            let sessions: Vec<Session> = lib
                .library
                .books
                .iter()
                .find(|b| b.url == *book_url)
                .map(|b| b.sessions.clone())
                .unwrap_or_default();

            let session_label = |s: &Session| {
                let ts = if s.updated_at.len() >= 16 {
                    let d = &s.updated_at[..10];
                    let h = &s.updated_at[11..13];
                    let m = &s.updated_at[14..16];
                    format!("{}@{}{}", d, h, m)
                } else {
                    s.updated_at.clone()
                };
                format!(
                    "{:20} {:>4}/{}  {}",
                    s.name,
                    (s.progress.current + 1).min(s.progress.total),
                    s.progress.total,
                    ts,
                )
            };

            let hints = if pending_delete_url.is_some() {
                " Press d again to confirm deletion  [Esc] Cancel "
            } else if input.is_some() {
                " [Enter] Confirm  [Esc] Cancel "
            } else {
                " [Enter] Open  [n] New  [r] Rename  [d] Delete  [↑↓] Move  [Esc] Cancel "
            };

            let items: Vec<ListItem> = if let Some(text) = input {
                if editing_id.is_some() {
                    sessions
                        .iter()
                        .enumerate()
                        .map(|(i, s)| {
                            let label = if i == *cursor {
                                format!("Rename: {}", text)
                            } else {
                                session_label(s)
                            };
                            ListItem::new(label)
                        })
                        .collect()
                } else {
                    let mut items: Vec<ListItem> = Vec::with_capacity(sessions.len() + 1);
                    items.push(ListItem::new(format!("Name: {}", text)));
                    for s in &sessions {
                        items.push(ListItem::new(session_label(s)));
                    }
                    items
                }
            } else {
                sessions
                    .iter()
                    .map(|s| ListItem::new(session_label(s)))
                    .collect()
            };

            let display_count = if input.is_some() && editing_id.is_none() {
                sessions.len() + 1
            } else {
                sessions.len()
            };

            let title = format!(
                " Sessions ({} Session{}) ",
                display_count,
                if display_count == 1 { "" } else { "s" }
            );

            draw_scrollable_list(
                frame,
                popup_area,
                title,
                items,
                *cursor,
                scroll_offset,
                hints,
            );
        }

        Modal::CommandPalette { .. } => {}

        Modal::InstallPlugin {
            url,
            cursor: _,
            scroll_offset,
        } => {
            let popup_area = centered_rect(60, 50, area);
            frame.render_widget(Clear, popup_area);

            let display_url = if url.is_empty() {
                "Enter GitHub repo URL...".to_string()
            } else {
                url.clone()
            };
            let items = vec![ListItem::new(display_url)];

            draw_scrollable_list(
                frame,
                popup_area,
                " Install Plugin from GitHub ".to_string(),
                items,
                0,
                scroll_offset,
                " [Enter] Install  [Esc] Cancel ",
            );
        }

        Modal::BackendPicker {
            cursor,
            scroll_offset,
            input,
            editing_idx: _,
            pending_delete_idx,
        } => {
            let popup_area = centered_rect(60, 50, area);
            frame.render_widget(Clear, popup_area);

            let backends: Vec<String> = lib
                .manager
                .backends
                .iter()
                .map(|b| b.name().to_string())
                .collect();

            let hints = if pending_delete_idx.is_some() {
                " Press d again to confirm deletion  [Esc] Cancel "
            } else if input.is_some() {
                " [Enter] Confirm  [Esc] Cancel "
            } else {
                " [Enter] Select  [n] New  [r] Rename  [d] Delete  [↑↓] Move  [Esc] Cancel "
            };

            let items: Vec<ListItem> = if let Some(text) = input {
                let mut items: Vec<ListItem> = Vec::with_capacity(backends.len() + 1);
                items.push(ListItem::new(format!("Name: {}", text)));
                for b in &backends {
                    items.push(ListItem::new(b.clone()));
                }
                items
            } else {
                backends.iter().map(|b| ListItem::new(b.clone())).collect()
            };

            let display_count = if input.is_some() {
                backends.len() + 1
            } else {
                backends.len()
            };
            let title = format!(
                " Backends ({} Backend{}) ",
                display_count,
                if display_count == 1 { "" } else { "s" }
            );

            draw_scrollable_list(
                frame,
                popup_area,
                title,
                items,
                *cursor,
                scroll_offset,
                hints,
            );
        }

        Modal::Filter {
            working,
            focus,
            tag_query,
            tag_cursor,
            tag_scroll,
            status_cursor,
            lib_cursor,
            ai_query,
        } => {
            let popup_area = centered_rect(60, 75, area);
            frame.render_widget(Clear, popup_area);

            // The bordered box holds only the facet rows + choice list; the
            // footer hint lines render BELOW the box, matching the other modals.
            let body_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(0), Constraint::Length(3)])
                .split(popup_area);
            let box_area = body_chunks[0];
            let footer_area = body_chunks[1];

            let block = Block::default().title(" Filter ").borders(Borders::ALL);
            let inner = block.inner(box_area);
            frame.render_widget(block, box_area);

            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(5), Constraint::Min(0)])
                .split(inner);

            let rows = [
                (FilterRow::Status, "Status", status_summary(working)),
                (FilterRow::Search, "Search", search_summary(working)),
                (FilterRow::Tags, "Tags", tags_summary(working)),
                (FilterRow::Library, "Library", library_summary(working)),
                (
                    FilterRow::Ai,
                    "AI",
                    if ai_query.trim().is_empty() {
                        "—".to_string()
                    } else {
                        ai_query.clone()
                    },
                ),
            ];

            let row_lines: Vec<Line> = rows
                .iter()
                .map(|(row, label, summary)| {
                    let focused = *focus == *row;
                    let marker = if focused { ">" } else { " " };
                    let label_style = if focused {
                        Style::default()
                            .fg(Color::Blue)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    };
                    let summary_style = if *row == FilterRow::Ai {
                        Style::default().fg(Color::DarkGray)
                    } else {
                        Style::default()
                    };
                    Line::from(vec![
                        Span::styled(format!("{} ", marker), label_style),
                        Span::styled(format!("{:<10}", label), label_style),
                        Span::styled(summary.clone(), summary_style),
                    ])
                })
                .collect();
            frame.render_widget(Paragraph::new(row_lines), chunks[0]);

            match focus {
                FilterRow::Tags => {
                    let tags = filter_tags(&known_tags(&lib.library.books), tag_query);
                    let items: Vec<ListItem> = tags
                        .iter()
                        .map(|t| {
                            let checked = working.tags.contains(t);
                            ListItem::new(format!("{} {}", if checked { "[x]" } else { "[ ]" }, t))
                        })
                        .collect();
                    render_choice_list(frame, chunks[1], items, *tag_cursor, tag_scroll);
                }
                FilterRow::Status => {
                    let items: Vec<ListItem> = STATUS_ROWS
                        .iter()
                        .map(|s| {
                            let label = match s {
                                Some(st) => st.to_string(),
                                None => "Any".to_string(),
                            };
                            let selected = working.status == *s;
                            ListItem::new(format!("{} {}", if selected { "•" } else { " " }, label))
                        })
                        .collect();
                    let mut scroll = 0;
                    render_choice_list(frame, chunks[1], items, *status_cursor, &mut scroll);
                }
                FilterRow::Library => {
                    let names = lib.manager.backend_names();
                    let mut items = vec![ListItem::new(format!(
                        "{} All",
                        if working.library.is_none() {
                            "•"
                        } else {
                            " "
                        }
                    ))];
                    for name in &names {
                        let selected = working.library.as_deref() == Some(name.as_str());
                        items.push(ListItem::new(format!(
                            "{} {}",
                            if selected { "•" } else { " " },
                            name
                        )));
                    }
                    let mut scroll = 0;
                    render_choice_list(frame, chunks[1], items, *lib_cursor, &mut scroll);
                }
                _ => {}
            }

            let n = lib.library.search(working).len();
            let footer_style = if n == 0 {
                Style::default().fg(Color::Red)
            } else {
                Style::default().fg(Color::DarkGray)
            };

            let footer_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(1),
                    Constraint::Length(1),
                    Constraint::Length(1),
                ])
                .split(footer_area);

            // Match count + apply/cancel, always visible.
            frame.render_widget(
                Paragraph::new(format!("{} matches — Enter Apply · Esc Cancel", n))
                    .style(footer_style),
                footer_chunks[0],
            );

            // Focus-aware action and nav hints, styled like the other modals'
            // footers (DarkGray `[Key] Label` lines). On Search and Ai, `c`
            // and space type into the text, so they are only offered on the
            // choice rows.
            let (action_hint, nav_hint): (&str, &str) = match *focus {
                FilterRow::Search => (
                    "[Type] Name  [Backspace] Del",
                    "[Tab] Row  [Shift+Tab] Prev",
                ),
                FilterRow::Ai => (
                    "[Type] Query  [Backspace] Del",
                    "[Tab] Row  [Shift+Tab] Prev",
                ),
                FilterRow::Status | FilterRow::Tags | FilterRow::Library => (
                    "[Space] Toggle  [c] Clear",
                    "[↑↓] Move  [Tab] Row  [Shift+Tab] Prev",
                ),
            };
            let hint_style = Style::default().fg(Color::DarkGray);
            frame.render_widget(
                Paragraph::new(action_hint).style(hint_style),
                footer_chunks[1],
            );
            frame.render_widget(Paragraph::new(nav_hint).style(hint_style), footer_chunks[2]);
        }

        Modal::ChapterResults {
            query,
            groups,
            cursor,
            scroll_offset,
            status,
        } => {
            let popup_area = centered_rect(65, 80, area);
            frame.render_widget(Clear, popup_area);

            // The bordered box holds only the results; the footer hint line
            // renders BELOW the box, matching the other modals.
            let body_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(0), Constraint::Length(1)])
                .split(popup_area);
            let box_area = body_chunks[0];
            let footer_area = body_chunks[1];

            let total_hits: usize = groups.iter().map(|g| g.chapters.len()).sum();
            let title = format!(" AI: \"{}\" ({}) ", query, total_hits);
            let block = Block::default().title(title).borders(Borders::ALL);
            let inner = block.inner(box_area);
            frame.render_widget(block, box_area);

            match status {
                SearchStatus::Loading => {
                    let para = Paragraph::new("searching…").alignment(Alignment::Center);
                    frame.render_widget(para, inner);
                }
                SearchStatus::Empty => {
                    let para = Paragraph::new(
                        "No matches — try rephrasing or fewer details.\n\nf refine · Esc close",
                    )
                    .alignment(Alignment::Center);
                    frame.render_widget(para, inner);
                }
                SearchStatus::Error(msg) => {
                    let para =
                        Paragraph::new(format!("Search failed: {}\n\nf retry · Esc close", msg))
                            .alignment(Alignment::Center);
                    frame.render_widget(para, inner);
                }
                SearchStatus::Ready => {
                    let (items, selected) = build_chapter_rows(groups, *cursor);
                    let visible_height = inner.height.saturating_sub(2) as usize;
                    if *cursor < *scroll_offset {
                        *scroll_offset = *cursor;
                    } else if *cursor >= *scroll_offset + visible_height {
                        *scroll_offset = *cursor - visible_height + 1;
                    }
                    let mut list_state = ListState::default();
                    *list_state.offset_mut() = *scroll_offset;
                    list_state.select(selected);
                    let list = List::new(items)
                        .highlight_style(Style::default().bg(Color::Blue).fg(Color::White))
                        .highlight_symbol("> ");
                    frame.render_stateful_widget(list, inner, &mut list_state);
                }
            }

            frame.render_widget(
                Paragraph::new(" ↑↓ move  Enter open  f refine  Esc close ")
                    .style(Style::default().fg(Color::DarkGray)),
                footer_area,
            );
        }

        Modal::EmbedChapters {
            book_url,
            chapters,
            selected,
            embedded_urls,
            cursor,
            scroll_offset,
        } => {
            let popup_area = centered_rect(70, 80, area);
            frame.render_widget(Clear, popup_area);

            // The bordered box holds only the checkbox list; the footer hint
            // line renders BELOW the box, matching the other modals.
            let body_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(0), Constraint::Length(1)])
                .split(popup_area);
            let box_area = body_chunks[0];
            let footer_area = body_chunks[1];

            let book_title = lib
                .library
                .books
                .iter()
                .find(|b| b.url == *book_url)
                .map(|b| b.title.clone())
                .unwrap_or_default();
            let selected_count = selected.iter().filter(|&&s| s).count();
            let title = format!(
                " Embed Chapters ({} selected) — {} ",
                selected_count, book_title
            );
            let block = Block::default().title(title).borders(Borders::ALL);
            let inner = block.inner(box_area);
            frame.render_widget(block, box_area);

            // Auto-scroll so the cursor stays visible.
            let visible_height = inner.height.saturating_sub(2) as usize;
            if *cursor < *scroll_offset {
                *scroll_offset = *cursor;
            } else if *cursor >= *scroll_offset + visible_height {
                *scroll_offset = *cursor - visible_height + 1;
            }

            let items: Vec<ListItem> = chapters
                .iter()
                .zip(selected.iter())
                .map(|(ch, &sel)| {
                    if embedded_urls.contains(&ch.url) {
                        // Already embedded — dimmed, not selectable.
                        ListItem::new(Line::from(vec![Span::styled(
                            format!(" [✓] Ch {}: {}", ch.order, ch.title),
                            Style::default().fg(Color::DarkGray),
                        )]))
                    } else {
                        let marker = if sel { "[x]" } else { "[ ]" };
                        ListItem::new(format!(" {} Ch {}: {}", marker, ch.order, ch.title))
                    }
                })
                .collect();

            let mut list_state = ListState::default();
            *list_state.offset_mut() = *scroll_offset;
            list_state.select(Some(*cursor));
            let list = List::new(items)
                .highlight_style(Style::default().bg(Color::Blue).fg(Color::White))
                .highlight_symbol("> ");
            frame.render_stateful_widget(list, inner, &mut list_state);

            frame.render_widget(
                Paragraph::new(" ↑↓ move  Space toggle  a all  c clear  Enter embed  Esc close ")
                    .style(Style::default().fg(Color::DarkGray)),
                footer_area,
            );
        }
    }
}

const STATUS_ROWS: [Option<BookStatus>; 5] = [
    None,
    Some(BookStatus::Reading),
    Some(BookStatus::Paused),
    Some(BookStatus::Dropped),
    Some(BookStatus::Completed),
];

/// Build the list items for the chapter-results modal. Book headers are
/// non-selectable; the returned `selected` is the visual row of the cursor
/// chapter (None when there are no chapters).
fn build_chapter_rows(
    groups: &[ChapterGroup],
    cursor: usize,
) -> (Vec<ListItem<'_>>, Option<usize>) {
    let mut items: Vec<ListItem> = Vec::new();
    let mut visual_of_flat: Vec<usize> = Vec::new();
    for group in groups {
        let genres = if group.genres.is_empty() {
            String::new()
        } else {
            format!(" [{}]", group.genres.join(", "))
        };
        // Pad the title so the genres and the right-aligned best score line up.
        let title_pad = 30usize.saturating_sub(group.book_title.chars().count());
        items.push(ListItem::new(Line::from(vec![
            Span::styled(
                format!(" {}{}", group.book_title, " ".repeat(title_pad)),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(genres, Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{:>8}", format!("{:.2}", group.best_score)),
                Style::default().fg(Color::DarkGray),
            ),
        ])));
        for hit in &group.chapters {
            visual_of_flat.push(items.len());
            let score = format!("{:>5.2}", hit.score);
            items.push(ListItem::new(Line::from(vec![
                Span::raw(format!("  {}", hit.chapter_title)),
                Span::styled(format!("  {}", score), Style::default().fg(Color::DarkGray)),
            ])));
        }
    }
    let selected = visual_of_flat.get(cursor).copied();
    (items, selected)
}

fn status_summary(filter: &BookFilter) -> String {
    match &filter.status {
        Some(status) => status.to_string(),
        None => "Any".to_string(),
    }
}

fn search_summary(filter: &BookFilter) -> String {
    if filter.name.is_empty() {
        "—".to_string()
    } else {
        filter.name.clone()
    }
}

fn tags_summary(filter: &BookFilter) -> String {
    if filter.tags.is_empty() {
        "—".to_string()
    } else {
        filter
            .tags
            .iter()
            .map(|t| format!("#{}", t))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

fn library_summary(filter: &BookFilter) -> String {
    match &filter.library {
        Some(name) => name.clone(),
        None => "All".to_string(),
    }
}

fn render_choice_list(
    frame: &mut Frame,
    area: Rect,
    items: Vec<ListItem>,
    cursor: usize,
    scroll: &mut usize,
) {
    let visible_height = area.height as usize;
    if cursor < *scroll {
        *scroll = cursor;
    } else if cursor >= *scroll + visible_height {
        *scroll = cursor - visible_height + 1;
    }

    let mut list_state = ListState::default();
    *list_state.offset_mut() = *scroll;
    list_state.select(Some(cursor));

    let list = List::new(items)
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
        .highlight_symbol("> ");

    frame.render_stateful_widget(list, area, &mut list_state);
}

fn draw_scrollable_list(
    frame: &mut Frame,
    area: Rect,
    title: String,
    items: Vec<ListItem>,
    cursor: usize,
    scroll_offset: &mut usize,
    hints: &str,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(area);

    let main_area = chunks[0];
    let visible_height = main_area.height.saturating_sub(2) as usize;

    if cursor < *scroll_offset {
        *scroll_offset = cursor;
    } else if cursor >= *scroll_offset + visible_height {
        *scroll_offset = cursor - visible_height + 1;
    }

    let mut list_state = ListState::default();
    *list_state.offset_mut() = *scroll_offset;
    list_state.select(Some(cursor));

    let list = List::new(items)
        .block(Block::default().title(title).borders(Borders::ALL))
        .highlight_symbol("> ")
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    frame.render_stateful_widget(list, main_area, &mut list_state);

    let hints = Paragraph::new(hints).style(Style::default().fg(Color::DarkGray));
    frame.render_widget(hints, chunks[1]);
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

    fn draw_modal_with(state: &mut AppState) {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                draw_modal(f, f.area(), &mut state.ui, &mut state.lib);
            })
            .unwrap();
    }

    #[test]
    fn test_draw_modal_command_palette_renders() {
        let mut state = make_state();
        state.ui.modal = Modal::CommandPalette {
            query: "test".into(),
            filtered: vec![],
            selected: 0,
        };
        draw_modal_with(&mut state);
    }

    #[test]
    fn test_draw_modal_add_book_renders() {
        let mut state = make_state();
        state.ui.modal = Modal::AddBook {
            inputs: vec!["http://example.com".into()],
            cursor: 0,
            scroll_offset: 0,
        };
        draw_modal_with(&mut state);
    }

    #[test]
    fn test_draw_modal_jump_chapter_renders() {
        let mut state = make_state();
        state.ui.modal = Modal::JumpChapter {
            chapters: vec![],
            query: "search".into(),
            filtered: vec![],
            cursor: 0,
            scroll_offset: 0,
            show_titles: true,
        };
        draw_modal_with(&mut state);
    }

    #[test]
    fn test_draw_modal_session_picker_renders() {
        let mut state = make_state();
        state
            .lib
            .library
            .add_book("Test Book".into(), "http://example.com/book".into());
        state.ui.modal = Modal::SessionPicker {
            book_url: "http://example.com/book".into(),
            cursor: 0,
            scroll_offset: 0,
            input: None,
            editing_id: None,
            pending_delete_url: None,
        };
        draw_modal_with(&mut state);
    }

    #[test]
    fn test_draw_modal_session_picker_pending_delete_renders() {
        let mut state = make_state();
        state
            .lib
            .library
            .add_book("Test Book".into(), "http://example.com/book".into());
        state.ui.modal = Modal::SessionPicker {
            book_url: "http://example.com/book".into(),
            cursor: 0,
            scroll_offset: 0,
            input: None,
            editing_id: None,
            pending_delete_url: Some("http://example.com/confirm-delete".into()),
        };
        draw_modal_with(&mut state);
    }

    #[test]
    fn test_draw_modal_none_does_not_panic() {
        let mut state = make_state();
        state.ui.modal = Modal::None;
        draw_modal_with(&mut state);
    }

    #[test]
    fn test_draw_modal_filter_renders() {
        let mut state = make_state();
        state
            .lib
            .library
            .add_book("Test Book".into(), "http://example.com/book".into());
        state.ui.modal = Modal::Filter {
            working: BookFilter::default(),
            focus: FilterRow::Search,
            tag_query: String::new(),
            tag_cursor: 0,
            tag_scroll: 0,
            status_cursor: 0,
            lib_cursor: 0,
            ai_query: String::new(),
        };
        draw_modal_with(&mut state);
    }

    #[test]
    fn test_draw_modal_filter_tags_focus_renders_choice_list() {
        let mut state = make_state();
        state
            .lib
            .library
            .add_book("Test Book".into(), "http://example.com/book".into());
        state.lib.library.books[0].tags = vec!["fantasy".into(), "litrpg".into()];
        state.ui.modal = Modal::Filter {
            working: BookFilter::default(),
            focus: FilterRow::Tags,
            tag_query: String::new(),
            tag_cursor: 0,
            tag_scroll: 0,
            status_cursor: 0,
            lib_cursor: 0,
            ai_query: String::new(),
        };
        draw_modal_with(&mut state);
    }

    #[test]
    fn test_draw_modal_filter_zero_matches_renders() {
        let mut state = make_state();
        state.ui.modal = Modal::Filter {
            working: BookFilter {
                name: "nonexistent".into(),
                ..Default::default()
            },
            focus: FilterRow::Search,
            tag_query: String::new(),
            tag_cursor: 0,
            tag_scroll: 0,
            status_cursor: 0,
            lib_cursor: 0,
            ai_query: String::new(),
        };
        draw_modal_with(&mut state);
    }

    /// Render the filter modal with the given focus and return the full
    /// terminal buffer content for footer assertions.
    fn draw_filter_modal(focus: FilterRow) -> String {
        let mut state = make_state();
        state
            .lib
            .library
            .add_book("Test Book".into(), "http://example.com/book".into());
        state.ui.modal = Modal::Filter {
            working: BookFilter::default(),
            focus,
            tag_query: String::new(),
            tag_cursor: 0,
            tag_scroll: 0,
            status_cursor: 0,
            lib_cursor: 0,
            ai_query: String::new(),
        };
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                draw_modal(f, f.area(), &mut state.ui, &mut state.lib);
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        buf.content().iter().map(|c| c.symbol()).collect()
    }

    #[test]
    fn test_draw_modal_filter_search_footer_shows_typing_keys() {
        let content = draw_filter_modal(FilterRow::Search);
        assert!(content.contains("[Type]"), "footer: {}", content);
        assert!(content.contains("[Backspace]"));
        assert!(content.contains("[Tab] Row"));
        assert!(content.contains("[Shift+Tab] Prev"));
        // `c` and space type into the name on Search — no toggle/clear hints.
        assert!(!content.contains("[Space] Toggle"));
        assert!(!content.contains("[c] Clear"));
    }

    #[test]
    fn test_draw_modal_filter_choice_footer_shows_toggle_keys() {
        let content = draw_filter_modal(FilterRow::Status);
        assert!(content.contains("[Space] Toggle"), "footer: {}", content);
        assert!(content.contains("[c] Clear"));
        assert!(content.contains("[↑↓] Move"));
        assert!(content.contains("[Tab] Row"));
        assert!(!content.contains("[Type]"));
    }

    #[test]
    fn test_draw_modal_filter_ai_footer_shows_typing_keys() {
        let content = draw_filter_modal(FilterRow::Ai);
        assert!(content.contains("[Type]"), "footer: {}", content);
        assert!(content.contains("[Backspace]"));
        assert!(content.contains("[Tab] Row"));
        assert!(!content.contains("[Space] Toggle"));
        assert!(!content.contains("[c] Clear"));
    }

    #[test]
    fn test_draw_modal_filter_footer_shows_match_count() {
        let content = draw_filter_modal(FilterRow::Search);
        assert!(content.contains("matches"), "footer: {}", content);
    }

    fn draw_chapter_results(status: SearchStatus) -> String {
        let mut state = make_state();
        let groups = match status {
            SearchStatus::Ready => vec![ChapterGroup {
                book_url: "u1".into(),
                book_title: "Book A".into(),
                genres: vec!["Fantasy".into()],
                chapters: vec![crate::event_types::ChapterHit {
                    book_url: "u1".into(),
                    book_title: "Book A".into(),
                    chapter_url: "c1".into(),
                    chapter_idx: 0,
                    chapter_title: "Ch1".into(),
                    score: 0.9,
                    genres: vec![],
                }],
                best_score: 0.9,
            }],
            _ => vec![],
        };
        state.ui.modal = Modal::ChapterResults {
            query: "dragon".into(),
            groups,
            cursor: 0,
            scroll_offset: 0,
            status,
        };
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                draw_modal(f, f.area(), &mut state.ui, &mut state.lib);
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        buf.content().iter().map(|c| c.symbol()).collect()
    }

    #[test]
    fn test_draw_modal_chapter_results_loading() {
        let content = draw_chapter_results(SearchStatus::Loading);
        assert!(content.contains("searching"), "content: {}", content);
        assert!(content.contains("AI: \"dragon\""));
    }

    #[test]
    fn test_draw_modal_chapter_results_empty() {
        let content = draw_chapter_results(SearchStatus::Empty);
        assert!(content.contains("No matches"), "content: {}", content);
        assert!(content.contains("f refine"));
    }

    #[test]
    fn test_draw_modal_chapter_results_error() {
        let content = draw_chapter_results(SearchStatus::Error("boom".into()));
        assert!(
            content.contains("Search failed: boom"),
            "content: {}",
            content
        );
        assert!(content.contains("f retry"));
    }

    #[test]
    fn test_draw_modal_chapter_results_ready() {
        let content = draw_chapter_results(SearchStatus::Ready);
        assert!(content.contains("Book A"), "content: {}", content);
        assert!(content.contains("Fantasy"), "content: {}", content);
        assert!(content.contains("Ch1"), "content: {}", content);
        assert!(content.contains("0.90"), "content: {}", content);
        assert!(content.contains("↑↓ move"), "content: {}", content);
    }

    fn draw_embed_chapters() -> String {
        let mut state = make_state();
        state
            .lib
            .library
            .add_book("Test Book".into(), "http://example.com/book".into());
        state.lib.library.books[0].chapters = vec![
            crate::models::Chapter {
                title: "The Arena".into(),
                url: "c1".into(),
                order: 12,
            },
            crate::models::Chapter {
                title: "The Hunt".into(),
                url: "c2".into(),
                order: 13,
            },
        ];
        state.ui.modal = Modal::EmbedChapters {
            book_url: "http://example.com/book".into(),
            chapters: state.lib.library.books[0].chapters.clone(),
            selected: vec![true, false],
            embedded_urls: vec!["c2".into()],
            cursor: 0,
            scroll_offset: 0,
        };
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                draw_modal(f, f.area(), &mut state.ui, &mut state.lib);
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        buf.content().iter().map(|c| c.symbol()).collect()
    }

    #[test]
    fn test_draw_modal_embed_chapters_renders() {
        let content = draw_embed_chapters();
        assert!(
            content.contains("Embed Chapters (1 selected) — Test Book"),
            "content: {}",
            content
        );
        // Rows show the chapter number and title.
        assert!(
            content.contains("[x] Ch 12: The Arena"),
            "content: {}",
            content
        );
        // "The Hunt" (url "c2") is embedded — dimmed with a checkmark, not selectable.
        assert!(
            content.contains("[✓] Ch 13: The Hunt"),
            "content: {}",
            content
        );
        assert!(!content.contains("[ ] The Hunt"), "content: {}", content);
        assert!(content.contains("Ch 12"), "content: {}", content);
        assert!(content.contains("Ch 13"), "content: {}", content);
        assert!(content.contains("Space toggle"), "content: {}", content);
        assert!(content.contains("Enter embed"), "content: {}", content);
    }
}
