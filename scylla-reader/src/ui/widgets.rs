//! Reusable widget helpers — centered_rect for popup placement.

use ratatui::prelude::*;
use ratatui::style::Modifier;
use ratatui::text::Line;

pub const NAV_HINTS: &[(&str, &str)] = &[
    ("1", "Library"),
    ("2", "Reader"),
    ("8", "Jobs"),
    ("9", "Settings"),
    (":", "Command"),
    ("?", "Hide"),
    ("Esc", "Quit"),
];

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hint_line_basic() {
        let line = hint_line("Keys", &[("q", "quit")]);
        let spans: Vec<_> = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(spans[0].contains("Keys"));
        assert!(spans.iter().any(|s| *s == "[q]"));
        assert!(spans.iter().any(|s| *s == " quit"));
    }

    #[test]
    fn test_hint_line_multiple_pairs() {
        let line = hint_line("Nav", &[("j", "down"), ("k", "up")]);
        let spans: Vec<_> = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(spans.iter().any(|s| *s == "[j]"));
        assert!(spans.iter().any(|s| *s == " down"));
        assert!(spans.iter().any(|s| *s == "[k]"));
        assert!(spans.iter().any(|s| *s == " up"));
    }

    #[test]
    fn test_centered_rect_returns_rect() {
        let area = Rect::new(0, 0, 100, 100);
        let r = centered_rect(50, 50, area);
        assert!(r.x >= area.x);
        assert!(r.y >= area.y);
        assert!(r.width <= area.width);
        assert!(r.height <= area.height);
    }

    #[test]
    fn test_centered_rect_full_size() {
        let area = Rect::new(0, 0, 100, 100);
        let r = centered_rect(100, 100, area);
        assert_eq!(r, area);
    }

    #[test]
    fn test_centered_rect_center() {
        let area = Rect::new(0, 0, 100, 100);
        let r = centered_rect(50, 50, area);
        assert_eq!(r.x, 25);
        assert_eq!(r.y, 25);
        assert_eq!(r.width, 50);
        assert_eq!(r.height, 50);
    }
}
