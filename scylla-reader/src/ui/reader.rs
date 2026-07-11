//! Reader page renderer — paged and scrollable modes.

use crate::settings::ReaderMode;
use crate::state::{LibraryState, ReaderState, UiState};
use crate::ui::widgets::hint_line;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

pub fn draw(frame: &mut Frame, area: Rect, reader: &ReaderState, lib: &LibraryState, ui: &UiState) {
    if reader.loading {
        draw_loading(frame, area);
        return;
    }
    if lib.library.selected_book().is_none() {
        draw_no_book(frame, area);
        return;
    }
    match lib.settings.reader_mode {
        ReaderMode::Paged => draw_paged(frame, area, reader, lib, ui),
        ReaderMode::Scrollable => draw_scrollable(frame, area, reader, lib, ui),
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

fn draw_no_book(frame: &mut Frame, area: Rect) {
    let block = Block::default()
        .title(" No Book Selected ")
        .borders(Borders::ALL);
    let para = Paragraph::new("Select a book from the library.")
        .block(block)
        .alignment(Alignment::Center);
    frame.render_widget(para, area);
}

fn draw_paged(
    frame: &mut Frame,
    area: Rect,
    reader: &ReaderState,
    _lib: &LibraryState,
    ui: &UiState,
) {
    let hint_height: u16 = if ui.show_hints { 2 } else { 1 };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(hint_height),
        ])
        .split(area);

    let session_label = if reader.session_name.is_empty() {
        String::new()
    } else {
        format!(" [Session: {}]", reader.session_name)
    };
    let header = Paragraph::new(format!(
        " {} — Ch.{} {}{}",
        reader.book_title,
        reader.current_chapter_idx + 1,
        reader.chapter_title,
        session_label,
    ))
    .style(Style::default().fg(Color::Yellow));
    frame.render_widget(header, chunks[0]);

    let lines = reader.page_lines_wrapped(chunks[1].width, chunks[1].height);
    let content = lines.join("\n");
    let block = Block::default().borders(Borders::LEFT);
    let paragraph = Paragraph::new(content)
        .block(block)
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, chunks[1]);
    if ui.show_hints {
        let hint_area = chunks[2];
        let hint_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Length(1)])
            .split(hint_area);
        frame.render_widget(
            Paragraph::new(hint_line(
                "Book",
                &[("< >", "Change Chapter"), ("← →", "Change Page")],
            )),
            hint_chunks[0],
        );
        frame.render_widget(
            Paragraph::new(hint_line("Nav", crate::ui::widgets::NAV_HINTS)),
            hint_chunks[1],
        );
    }
}

fn draw_scrollable(
    frame: &mut Frame,
    area: Rect,
    reader: &ReaderState,
    _lib: &LibraryState,
    ui: &UiState,
) {
    let hint_height: u16 = if ui.show_hints { 2 } else { 1 };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(hint_height),
        ])
        .split(area);

    let session_label = if reader.session_name.is_empty() {
        String::new()
    } else {
        format!(" [Session: {}]", reader.session_name)
    };
    let header = Paragraph::new(format!(
        " {} — Ch.{} {}{}",
        reader.book_title,
        reader.current_chapter_idx + 1,
        reader.chapter_title,
        session_label,
    ))
    .style(Style::default().fg(Color::Yellow));
    frame.render_widget(header, chunks[0]);

    let lines = reader.visible_wrapped_lines(chunks[1].width, chunks[1].height);
    let content = lines.join("\n");
    let block = Block::default().borders(Borders::LEFT);
    let paragraph = Paragraph::new(content).block(block);
    frame.render_widget(paragraph, chunks[1]);

    if ui.show_hints {
        let hint_area = chunks[2];
        let hint_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Length(1)])
            .split(hint_area);
        frame.render_widget(
            Paragraph::new(hint_line(
                "Book",
                &[("< >", "Change Chapter"), ("↑ ↓", "Scroll")],
            )),
            hint_chunks[0],
        );
        frame.render_widget(
            Paragraph::new(hint_line("Nav", crate::ui::widgets::NAV_HINTS)),
            hint_chunks[1],
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;
    use crate::library::Library;
    use crate::state::AppState;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn state_with_reader_content() -> AppState {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let db = Db::open_conn(conn).unwrap();
        let mut state = AppState::from_parts(db, Library::new());
        state.lib.library.add_book("Test Book".into(), "url".into());
        state.reader.load(
            "Test Book".into(),
            "url".into(),
            "Ch1".into(),
            "hello\nworld".into(),
            0,
        );
        state
    }

    #[test]
    fn test_reader_draw_shows_hints_when_enabled() {
        let mut state = state_with_reader_content();
        state.ui.show_hints = true;

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                draw(f, f.area(), &state.reader, &state.lib, &state.ui);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        let content: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(content.contains("[1]"));
    }

    #[test]
    fn test_reader_draw_shows_content() {
        let mut state = state_with_reader_content();
        state.ui.show_hints = false;

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                draw(f, f.area(), &state.reader, &state.lib, &state.ui);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        let content: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(content.contains("hello"));
        assert!(content.contains("world"));
    }

    #[test]
    fn test_reader_draw_hides_hints_when_disabled() {
        let mut state = state_with_reader_content();
        state.ui.show_hints = false;

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                draw(f, f.area(), &state.reader, &state.lib, &state.ui);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        let content: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(!content.contains("[1]"));
    }
}
