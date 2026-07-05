//! Reusable widget helpers — centered_rect for popup placement.

use ratatui::prelude::*;
use ratatui::style::Modifier;
use ratatui::text::Line;

pub fn hint_line<'a>(category: &str, pairs: &[(&'a str, &'a str)]) -> Line<'a> {
    let mut spans = vec![
        Span::styled(
            format!(" {} ", category),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
    ];
    for (i, (key, label)) in pairs.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw("  "));
        }
        spans.push(Span::styled(
            format!("[{}]", key),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::raw(format!(" {}", label)));
    }
    Line::from(spans)
}

pub fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
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
