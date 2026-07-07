//! Library page renderer — book list, detail side panel, filter bar.

use crate::state::AppState;

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};
use ratatui_image::StatefulImage;

use crate::ui::widgets::hint_line;

pub fn draw(frame: &mut Frame, area: Rect, state: &mut AppState) {
    let hint_height: u16 = if state.show_hints { 2 } else { 1 };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(hint_height),
        ])
        .split(area);

    let filter_bar = Paragraph::new(format!(" Filter: {}", state.library.filter))
        .style(Style::default().fg(Color::Yellow));
    frame.render_widget(filter_bar, chunks[0]);

    let main_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(chunks[1]);

    draw_book_list(frame, main_chunks[0], state);
    draw_side_panel(frame, main_chunks[1], state);

    if state.show_hints {
        let hint_area = chunks[2];
        let hint_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Length(1)])
            .split(hint_area);
        frame.render_widget(
            Paragraph::new(hint_line(
                "Actions",
                &[
                    ("i", "Add"),
                    ("j", "Jump"),
                    ("d", "Delete"),
                    ("u", "Update"),
                    ("f", "Filter"),
                    ("Space", "Status"),
                ],
            )),
            hint_chunks[0],
        );
        frame.render_widget(
            Paragraph::new(hint_line("Nav", crate::ui::widgets::NAV_HINTS)),
            hint_chunks[1],
        );
    }

    crate::ui::modal::draw_modal(frame, area, state);
}

fn draw_book_list(frame: &mut Frame, area: Rect, state: &mut AppState) {
    let block = Block::default().title(" Library ").borders(Borders::ALL);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let visible = state.library.visible_indices();
    let items: Vec<ListItem> = visible
        .iter()
        .map(|&i| {
            let b = &state.library.books[i];
            let tags = if b.tags.is_empty() {
                String::new()
            } else {
                format!(" [{}]", b.tags.join(", "))
            };
            let session_info = if b.sessions.len() > 1 {
                format!(" [{} sessions]", b.sessions.len())
            } else {
                String::new()
            };
            ListItem::new(format!(
                "{} ({}) {}/{}{}{}",
                b.title,
                b.status,
                if b.sessions.first().map(|s| s.progress.total).unwrap_or(0) == 0 {
                    0
                } else {
                    let active = b
                        .active_session_id
                        .and_then(|id| b.sessions.iter().find(|s| s.id == id))
                        .or_else(|| b.sessions.first());
                    match active {
                        Some(s) => (s.progress.current + 1).min(s.progress.total),
                        None => 0,
                    }
                },
                b.sessions.first().map(|s| s.progress.total).unwrap_or(0),
                tags,
                session_info,
            ))
        })
        .collect();

    let mut list_state = ListState::default();
    list_state.select(Some(state.library.selected_index));

    let list = List::new(items)
        .highlight_style(Style::default().bg(Color::Blue).fg(Color::White))
        .highlight_symbol(">> ");

    frame.render_stateful_widget(list, inner, &mut list_state);
}

fn draw_side_panel(frame: &mut Frame, area: Rect, state: &mut AppState) {
    let block = Block::default().title(" Details ").borders(Borders::ALL);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let book_data = state.library.selected_book().map(|b| {
        (
            b.title.clone(),
            b.status.clone(),
            b.sessions.clone(),
            b.active_session_id,
            b.tags.clone(),
            b.description.clone(),
            b.cover_url.clone(),
        )
    });

    let Some((title, status, sessions, active_session_id, tags, description, cover_url)) =
        book_data
    else {
        frame.render_widget(Paragraph::new("No book selected"), inner);
        return;
    };

    let side_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(12), Constraint::Min(0)])
        .split(inner);

    if let Some(url) = &cover_url {
        if let Some(protocol) = state.library.cover_cache.get_mut(url) {
            frame.render_stateful_widget(StatefulImage::new(None), side_chunks[0], protocol);
        }
    }

    let active_session = active_session_id
        .and_then(|id| sessions.iter().find(|s| s.id == id))
        .or_else(|| sessions.first());

    let progress_str = match active_session {
        Some(s) => format!(
            "{}/{} chapters [Session: {}]",
            if s.progress.total == 0 {
                0
            } else {
                (s.progress.current + 1).min(s.progress.total)
            },
            s.progress.total,
            s.name,
        ),
        None => "No sessions".to_string(),
    };

    let session_count = format!("Sessions:  {}", sessions.len());

    let details = format!(
        "Title:    {}\nStatus:   {}\nProgress: {}\n{}\nTags:     {}\n\n{}",
        title,
        status,
        progress_str,
        session_count,
        tags.join(", "),
        description.unwrap_or_default(),
    );
    frame.render_widget(
        Paragraph::new(details).wrap(Wrap { trim: false }),
        side_chunks[1],
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;
    use crate::library::Library;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    #[test]
    fn test_library_draw_shows_hints_when_enabled() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let db = Db::open_conn(conn).unwrap();
        let mut state = AppState::from_parts(db, Library::new());
        state.show_hints = true;

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                draw(f, f.area(), &mut state);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        let content: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(content.contains("[1]"));
    }

    #[test]
    fn test_library_draw_shows_book_titles() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let db = Db::open_conn(conn).unwrap();
        let mut state = AppState::from_parts(db, Library::new());
        state
            .library
            .add_book("Test Book Title".into(), "url".into(), 10);

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                draw(f, f.area(), &mut state);
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        let content: String = buffer.content().iter().map(|c| c.symbol()).collect();
        assert!(content.contains("Test Book Title"));
    }

    #[test]
    fn test_library_draw_hides_hints_when_disabled() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let db = Db::open_conn(conn).unwrap();
        let mut state = AppState::from_parts(db, Library::new());
        state.show_hints = false;

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                draw(f, f.area(), &mut state);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        let content: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(!content.contains("[1]"));
    }
}
