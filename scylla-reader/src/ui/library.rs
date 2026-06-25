use crate::app::AppState;

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};
use ratatui_image::StatefulImage;

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
            ListItem::new(format!(
                "{} ({}) {}/{}{}",
                b.title, b.status, b.progress.current, b.progress.total, tags
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
            b.progress.clone(),
            b.tags.clone(),
            b.description.clone(),
        )
    });

    let Some((title, status, progress, tags, description)) = book_data else {
        frame.render_widget(Paragraph::new("No book selected"), inner);
        return;
    };

    let side_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(12), Constraint::Min(0)])
        .split(inner);

    if let Some(protocol) = &mut state.library.cached_protocol {
        frame.render_stateful_widget(StatefulImage::new(None), side_chunks[0], protocol);
    }

    let details = format!(
        "Title:    {}\nStatus:   {}\nProgress: {}/{} chapters\nTags:     {}\n\n{}",
        title,
        status,
        progress.current,
        progress.total,
        tags.join(", "),
        description.unwrap_or_default(),
    );

    frame.render_widget(
        Paragraph::new(details).wrap(Wrap { trim: false }),
        side_chunks[1],
    );
}
