//! Modal popup renderer — add-book form and jump-to-chapter list.

use crate::models::Session;
use crate::state::modal::Modal;
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
            cursor: _cursor,
            scroll_offset: _,
        } => {
            let popup_area = centered_rect(70, 5, area);
            frame.render_widget(Clear, popup_area);

            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(3), Constraint::Length(1)])
                .split(popup_area);
            let main_area = chunks[0];

            let input_text = if url.is_empty() {
                "Enter GitHub repo URL...".to_string()
            } else {
                url.clone()
            };

            let input = Paragraph::new(input_text)
                .block(
                    Block::default()
                        .title(" Install Plugin from GitHub ")
                        .borders(Borders::ALL),
                )
                .style(Style::default().fg(Color::White));
            frame.render_widget(input, main_area);

            let hints = Paragraph::new(" [Enter] Install  [Esc] Cancel ")
                .style(Style::default().fg(Color::DarkGray));
            frame.render_widget(hints, chunks[1]);
        }
    }
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
    use crate::db::Db;
    use crate::library::Library;
    use crate::state::AppState;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn make_state() -> AppState {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let db = Db::open_conn(conn).unwrap();
        AppState::from_parts(db, Library::new())
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
        state.lib.library.add_book("Test Book".into(), "http://example.com/book".into());
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
        state.lib.library.add_book("Test Book".into(), "http://example.com/book".into());
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
}