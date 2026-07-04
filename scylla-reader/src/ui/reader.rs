//! Reader page renderer — paged and scrollable modes.

use crate::settings::ReaderMode;
use crate::state::AppState;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

pub fn draw(frame: &mut Frame, area: Rect, state: &AppState) {
    if state.reader.loading {
        draw_loading(frame, area);
        return;
    }
    match state.settings.reader_mode {
        ReaderMode::Paged => draw_paged(frame, area, state),
        ReaderMode::Scrollable => draw_scrollable(frame, area, state),
    }
}

fn draw_loading(frame: &mut Frame, area: Rect) {
    let block = Block::default()
        .title(" Loading Chapter... ")
        .borders(Borders::ALL);
    let para = Paragraph::new("Fetching chapter content, please wait...")
        .block(block)
        .alignment(Alignment::Center);
    frame.render_widget(para, area);
}

fn draw_paged(frame: &mut Frame, area: Rect, state: &AppState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(area);

    let header = Paragraph::new(format!(
        " {} — Ch.{} {}",
        state.reader.book_title,
        state.reader.current_chapter_idx + 1,
        state.reader.chapter_title,
    ))
    .style(Style::default().fg(Color::Yellow));
    frame.render_widget(header, chunks[0]);

    let lines = state
        .reader
        .page_lines_wrapped(chunks[1].width, chunks[1].height);
    let content = lines.join("\n");
    let block = Block::default().borders(Borders::LEFT);
    let paragraph = Paragraph::new(content)
        .block(block)
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, chunks[1]);
    if state.show_hints {
        let hints = Paragraph::new(" > Next  < Prev  h/l Page  j/k Scroll  PgDn/PgUp  Esc Back  : Palette")
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(hints, chunks[2]);
    }
}

fn draw_scrollable(frame: &mut Frame, area: Rect, state: &AppState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(area);

    let header = Paragraph::new(format!(
        " {} — Ch.{} {}",
        state.reader.book_title,
        state.reader.current_chapter_idx + 1,
        state.reader.chapter_title,
    ))
    .style(Style::default().fg(Color::Yellow));
    frame.render_widget(header, chunks[0]);

    let lines = state
        .reader
        .visible_wrapped_lines(chunks[1].width, chunks[1].height);
    let content = lines.join("\n");
    let block = Block::default().borders(Borders::LEFT);
    let paragraph = Paragraph::new(content).block(block);
    frame.render_widget(paragraph, chunks[1]);

    if state.show_hints {
        let hints = Paragraph::new(" > Next  < Prev  h/l Page  j/k Scroll  PgDn/PgUp  Esc Back  : Palette")
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(hints, chunks[2]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;
    use crate::library::Library;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    #[test]
    fn test_reader_draw_shows_hints_when_enabled() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let db = Db::open_conn(conn).unwrap();
        let mut state = AppState::from_parts(db, Library::new());
        state.show_hints = true;

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                draw(f, f.area(), &state);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        let content: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(content.contains(": Palette"));
    }

    #[test]
    fn test_reader_draw_hides_hints_when_disabled() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let db = Db::open_conn(conn).unwrap();
        let mut state = AppState::from_parts(db, Library::new());
        state.show_hints = false;

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                draw(f, f.area(), &state);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        let content: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(!content.contains("Palette"));
    }
}
