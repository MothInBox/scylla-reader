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
        state.reader.book_title, state.reader.current_chapter_idx, state.reader.chapter_title,
    ))
    .style(Style::default().fg(Color::Yellow));
    frame.render_widget(header, chunks[0]);

    // Calculate wrapped visual lines for the current page using area width/height.
    let lines = state
        .reader
        .page_lines_wrapped(chunks[1].width, chunks[1].height);
    let content = lines.join("\n");
    let block = Block::default().borders(Borders::LEFT);
    let paragraph = Paragraph::new(content)
        .block(block)
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, chunks[1]);
    let total = state
        .reader
        .total_pages_for(chunks[1].width, chunks[1].height);
    let show_page_controls = if total > 1 { true } else { false };
    let prev_hint = if state.reader.current_chapter_idx > 0 {
        "[<] Prev Chapter  "
    } else {
        ""
    };
    let next_hint = if state
        .library
        .selected_book()
        .map(|b| state.reader.current_chapter_idx + 1 < b.chapters.len())
        .unwrap_or(false)
    {
        "  [>] Next Chapter"
    } else {
        ""
    };
    let page_ctrls = if show_page_controls {
        format!(
            "Page {}/{}  [h/←] [l/→] Turn Page",
            state.reader.page + 1,
            total
        )
    } else {
        String::new()
    };
    let footer = Paragraph::new(format!(
        " {}{}{}  [Esc] Library",
        prev_hint, page_ctrls, next_hint,
    ))
    .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(footer, chunks[2]);
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
        state.reader.book_title, state.reader.current_chapter_idx, state.reader.chapter_title,
    ))
    .style(Style::default().fg(Color::Yellow));
    frame.render_widget(header, chunks[0]);

    let visible_lines: Vec<&str> = state
        .reader
        .content
        .iter()
        .skip(state.reader.scroll)
        .map(|s| s.as_str())
        .collect();
    let content = visible_lines.join("\n");
    let block = Block::default().borders(Borders::LEFT);
    let paragraph = Paragraph::new(content)
        .block(block)
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, chunks[1]);

    let progress = if state.reader.content.is_empty() {
        0
    } else {
        (state.reader.scroll * 100) / state.reader.content.len()
    };
    let prev_hint = if state.reader.current_chapter_idx > 0 {
        "[<] Prev  "
    } else {
        ""
    };
    let next_hint = if state
        .library
        .selected_book()
        .map(|b| state.reader.current_chapter_idx + 1 < b.chapters.len())
        .unwrap_or(false)
    {
        "  [>] Next"
    } else {
        ""
    };
    let footer = Paragraph::new(format!(
        " {}{}%  [j/↓] [k/↑] Scroll  [PgDn/PgUp]{}  [Esc] Library",
        prev_hint, progress, next_hint,
    ))
    .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(footer, chunks[2]);
}
