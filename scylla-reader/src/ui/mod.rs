pub mod library;
pub mod reader;
pub mod settings;

use crate::app::{AppState, Page};
use ratatui::prelude::*;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph};

pub fn draw(frame: &mut Frame, state: &mut AppState, area: Rect) {
    match state.current_page {
        Page::Library => library::draw(frame, area, state),
        Page::Settings => settings::draw(frame, area, state),
        Page::Reader => reader::draw(frame, area, state),
        Page::AddingBook => {
            library::draw(frame, area, state);
        }
        Page::BookChapterJump => library::draw(frame, area, state),
    }
    if state.current_page == Page::AddingBook {
        let popup_area = centered_rect(70, 50, area);
        frame.render_widget(Clear, popup_area);
        draw_input_widget(frame, popup_area, state);
    }
    if state.current_page == Page::BookChapterJump {
        let popup_area = centered_rect(70, 50, area);
        frame.render_widget(Clear, popup_area);
        draw_jump_widget(frame, popup_area, state);
    }
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
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

fn draw_input_widget(frame: &mut Frame, area: Rect, state: &mut AppState) {
    let count = state.win_inputs.len();
    let items = state
        .win_inputs
        .iter()
        .map(|l| ListItem::new(l.to_string()))
        .collect();
    draw_scrollable_list(
        frame,
        area,
        format!(
            " Add Books ({} URL{}) ",
            count,
            if count == 1 { "" } else { "s" }
        ),
        items,
        state.win_cursor,
        &mut state.win_scroll_offset,
        " [Enter] New line  [Ctrl+s] Submit all  [↑↓] Move  [Backspace] Delete  [Esc] Cancel",
    );
}

fn draw_jump_widget(frame: &mut Frame, area: Rect, state: &mut AppState) {
    let count = state.win_inputs.len();
    let items = state
        .win_inputs
        .iter()
        .map(|l| ListItem::new(l.to_string()))
        .collect();
    draw_scrollable_list(
        frame,
        area,
        format!(
            " Jump to Chapter ({} Chapter{}) ",
            count,
            if count == 1 { "" } else { "s" }
        ),
        items,
        state.win_cursor,
        &mut state.win_scroll_offset,
        " [Esc] Cancel",
    );
}
