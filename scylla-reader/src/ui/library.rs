use crate::app::AppState;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};
use ratatui_image::StatefulImage;
use ratatui_image::picker::Picker;

pub fn draw(frame: &mut Frame, area: Rect, state: &mut AppState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
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

    let hints = Paragraph::new(" [i] Add  [d] Delete [j] Jump  [Space] Status  [f] Filter  [u] Update  [Tab] Settings  [q] Quit")
        .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(hints, chunks[2]);
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
            ListItem::new(format!(
                "{} ({}) {}/{}{}",
                b.title, b.status, b.progress.current, b.progress.total, tags,
            ))
        })
        .collect();

    let list = List::new(items)
        .highlight_style(Style::default().bg(Color::Blue).fg(Color::White))
        .highlight_symbol(">> ");

    let mut list_state = ListState::default();
    list_state.select(Some(state.library.selected_index));
    frame.render_stateful_widget(list, inner, &mut list_state);
}

fn draw_side_panel(frame: &mut Frame, area: Rect, state: &mut AppState) {
    let block = Block::default().title(" Details ").borders(Borders::ALL);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Extract what we need immediately so the borrow ends here
    let Some((title, status, progress, tags, description)) =
        state.library.selected_book().map(|book| {
            (
                book.title.clone(),
                book.status.clone(),
                book.progress.clone(),
                book.tags.clone(),
                book.description.clone(),
            )
        })
    else {
        let empty = Paragraph::new("No book selected").style(Style::default().fg(Color::DarkGray));
        frame.render_widget(empty, inner);
        return;
    };

    let side_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(12), Constraint::Min(0)])
        .split(inner);

    // Mutable borrow now safe — no immutable borrow of state.library alive
    if let Some(protocol) = &mut state.library.cached_protocol {
        let image_widget = StatefulImage::new(None);
        frame.render_stateful_widget(image_widget, side_chunks[0], protocol);
    }

    let tags_str = if tags.is_empty() {
        "None".to_string()
    } else {
        tags.join(", ")
    };

    let desc = description
        .as_deref()
        .unwrap_or("No description available.");
    let desc = if desc.len() > 500 {
        format!("{}...", &desc[..500])
    } else {
        desc.to_string()
    };

    let details = format!(
        "Title:    {}\nStatus:   {}\nProgress: {}/{} chapters\nTags:     {}\n\n{}",
        title, status, progress.current, progress.total, tags_str, desc,
    );

    let details_widget = Paragraph::new(details).wrap(Wrap { trim: false });
    frame.render_widget(details_widget, side_chunks[1]);
}
