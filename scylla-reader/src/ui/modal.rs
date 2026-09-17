//! Modal popup renderer — add-book form and jump-to-chapter list.

use crate::library::{BookFilter, filter_tags, known_tags};
use crate::models::{BookStatus, Session};
use crate::state::modal::{FilterRow, Modal};
use crate::state::{LibraryState, UiState};
use crate::ui::palette::draw_palette;
use crate::ui::widgets::{centered_rect, hint_line};
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
        } => {
            let popup_area = centered_rect(60, 75, area);
            frame.render_widget(Clear, popup_area);

            let block = Block::default().title(" Filter ").borders(Borders::ALL);
            let inner = block.inner(popup_area);
            frame.render_widget(block, popup_area);

            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(5),
                    Constraint::Min(0),
                    Constraint::Length(3),
                ])
                .split(inner);

            let rows = [
                (FilterRow::Status, "Status", status_summary(working)),
                (FilterRow::Search, "Search", search_summary(working)),
                (FilterRow::Tags, "Tags", tags_summary(working)),
                (FilterRow::Library, "Library", library_summary(working)),
                (FilterRow::Ai, "AI", "— coming soon —".to_string()),
            ];

            let row_lines: Vec<Line> = rows
                .iter()
                .map(|(row, label, summary)| {
                    let focused = *focus == *row;
                    let marker = if focused { "▸" } else { " " };
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
                .split(chunks[2]);

            // Match count + apply/cancel, always visible.
            frame.render_widget(
                Paragraph::new(format!("{} matches — Enter Apply · Esc Cancel", n))
                    .style(footer_style),
                footer_chunks[0],
            );

            // Focus-aware action keys. On Search, `c` and space type into the
            // name, so they are only offered on the choice rows.
            let actions: &[(&str, &str)] = match *focus {
                FilterRow::Search => &[("Type", "Name"), ("Backspace", "Del")],
                FilterRow::Status | FilterRow::Tags | FilterRow::Library => {
                    &[("Space", "Toggle"), ("c", "Clear")]
                }
                FilterRow::Ai => &[],
            };

            // Row cycling always applies; ↑↓ moves only on the choice rows.
            let mut nav: Vec<(&str, &str)> = vec![("Tab", "Row"), ("Shift+Tab", "Prev")];
            if matches!(
                *focus,
                FilterRow::Status | FilterRow::Tags | FilterRow::Library
            ) {
                nav.push(("↑↓", "Move"));
            }

            // The AI row is inert, so it has no action keys of its own.
            if actions.is_empty() {
                frame.render_widget(Paragraph::new(hint_line("Nav", &nav)), footer_chunks[1]);
            } else {
                frame.render_widget(
                    Paragraph::new(hint_line("Actions", actions)),
                    footer_chunks[1],
                );
                frame.render_widget(Paragraph::new(hint_line("Nav", &nav)), footer_chunks[2]);
            }
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
    fn test_draw_modal_filter_ai_footer_shows_apply_cancel() {
        let content = draw_filter_modal(FilterRow::Ai);
        assert!(content.contains("[Tab] Row"), "footer: {}", content);
        assert!(!content.contains("[Space] Toggle"));
        assert!(!content.contains("[Type]"));
    }

    #[test]
    fn test_draw_modal_filter_footer_shows_match_count() {
        let content = draw_filter_modal(FilterRow::Search);
        assert!(content.contains("matches"), "footer: {}", content);
    }
}
