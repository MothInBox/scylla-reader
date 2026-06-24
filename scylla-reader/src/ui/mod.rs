pub mod library;
pub mod reader;
pub mod settings;

use crate::app::{AppState, Page};
use crate::models::book;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

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

fn draw_input_widget(frame: &mut Frame, area: Rect, state: &AppState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(area);

    let display: String = state
        .win_inputs
        .iter()
        .enumerate()
        .map(|(i, line)| {
            if i == state.win_cursor {
                format!("> {}", line)
            } else {
                format!("  {}", line)
            }
        })
        .collect::<Vec<_>>()
        .join("\n");

    let block = Block::default()
        .title(format!(
            " Add Books ({} URL{}) ",
            state.win_inputs.len(),
            if state.win_inputs.len() == 1 { "" } else { "s" }
        ))
        .borders(Borders::ALL);

    let paragraph = Paragraph::new(display)
        .block(block)
        .wrap(ratatui::widgets::Wrap { trim: false });
    frame.render_widget(paragraph, chunks[0]);

    let hints = Paragraph::new(" [Enter] New line  [Ctrl+s] Submit all  [↑↓] Move between lines  [Backspace] Delete  [Esc] Cancel")
        .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(hints, chunks[1]);
}

fn draw_jump_widget(frame: &mut Frame, area: Rect, state: &AppState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(area);

    let display: String = state
        .win_inputs
        .iter()
        .enumerate()
        .map(|(i, line)| {
            if i == state.win_cursor {
                format!("> {}", line)
            } else {
                format!("  {}", line)
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    let chapter_count = state.win_inputs.len();

    let block = Block::default()
        .title(format!(
            " Jump to Chapter ({} Chapter{}) ",
            chapter_count,
            if chapter_count == 1 { "" } else { "s" }
        ))
        .borders(Borders::ALL);
    let paragraph = Paragraph::new(display)
        .block(block)
        .wrap(ratatui::widgets::Wrap { trim: false });
    frame.render_widget(paragraph, chunks[0]);

    let hints = Paragraph::new("Esc] Cancel").style(Style::default().fg(Color::DarkGray));
    frame.render_widget(hints, chunks[1]);
}
