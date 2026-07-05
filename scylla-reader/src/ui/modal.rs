//! Modal popup renderer — add-book form and jump-to-chapter list.

use crate::models::Session;
use crate::state::AppState;
use crate::state::modal::Modal;
use crate::ui::palette::draw_palette;
use crate::ui::widgets::centered_rect;
use ratatui::prelude::*;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph};

pub fn draw_modal(frame: &mut Frame, area: Rect, state: &mut AppState) {
    if matches!(state.modal, Modal::CommandPalette { .. }) {
        draw_palette(frame, area, state);
        return;
    }

    match &mut state.modal {
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
            chapters,
            cursor,
            scroll_offset,
            show_titles,
        } => {
            let popup_area = centered_rect(70, 50, area);
            frame.render_widget(Clear, popup_area);

            let count = chapters.len();
            let items: Vec<ListItem> = chapters
                .iter()
                .map(|ch| {
                    ListItem::new(if *show_titles {
                        ch.title.clone()
                    } else {
                        ch.url.clone()
                    })
                })
                .collect();

            let title = format!(
                " Jump to Chapter ({} Chapter{}) ",
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
                " [Enter] Set Current  [↑↓] Move  [t] Link/Title  [Esc] Cancel",
            );
        }

        Modal::SessionPicker {
            book_url,
            cursor,
            scroll_offset,
            input,
            editing_id,
        } => {
            let popup_area = centered_rect(60, 50, area);
            frame.render_widget(Clear, popup_area);

            let sessions: Vec<Session> = state
                .library
                .books
                .iter()
                .find(|b| b.url == *book_url)
                .map(|b| b.sessions.clone())
                .unwrap_or_default();

            let count = sessions.len();

            let hints = if input.is_some() {
                " [Enter] Confirm  [Esc] Cancel "
            } else {
                " [Enter] Open  [n] New  [r] Rename  [d] Delete  [↑↓] Move  [Esc] Cancel "
            };

            let items: Vec<ListItem> = sessions
                .iter()
                .enumerate()
                .map(|(i, s)| {
                    let label = if let Some(text) = input {
                        if i == *cursor {
                            let prefix = if editing_id.is_some() { "Rename:" } else { "Name:" };
                            format!("{} {}", prefix, text)
                        } else {
                            format!(
                                "{} — {}/{}",
                                s.name,
                                (s.progress.current + 1).min(s.progress.total),
                                s.progress.total
                            )
                        }
                    } else {
                        format!(
                            "{} — {}/{}",
                            s.name,
                            (s.progress.current + 1).min(s.progress.total),
                            s.progress.total
                        )
                    };
                    ListItem::new(label)
                })
                .collect();

            let title = format!(
                " Sessions ({} Session{}) ",
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
                hints,
            );
        }

        Modal::CommandPalette { .. } => unreachable!(),
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
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    #[test]
    fn test_draw_modal_command_palette_renders() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let db = Db::open_conn(conn).unwrap();
        let mut state = AppState::from_parts(db, Library::new());
        state.modal = Modal::CommandPalette {
            query: "test".into(),
            filtered: vec![],
            selected: 0,
        };

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                draw_modal(f, f.area(), &mut state);
            })
            .unwrap();
    }
}
