use crate::app::AppState;
use crate::app::modal::Modal;
use crate::ui::widgets::centered_rect;
use ratatui::prelude::*;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph};

pub fn draw_modal(frame: &mut Frame, area: Rect, state: &mut AppState) {
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
        } => {
            let popup_area = centered_rect(70, 50, area);
            frame.render_widget(Clear, popup_area);

            let count = chapters.len();
            let items: Vec<ListItem> = chapters
                .iter()
                .map(|ch| ListItem::new(ch.url.clone()))
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
                " [Enter] Set Current  [↑↓] Move  [Esc] Cancel",
            );
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
