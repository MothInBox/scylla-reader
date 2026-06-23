use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use crate::app::AppState;
use crate::settings::{SettingsField, SettingsPage};

pub fn draw(frame: &mut Frame, area: Rect, state: &AppState) {
    match state.settings.settings_page {
        SettingsPage::Main => draw_main(frame, area, state),
        SettingsPage::CookieList => draw_cookie_list(frame, area, state),
        SettingsPage::CookieEdit => draw_cookie_edit(frame, area, state),
    }
}

fn draw_main(frame: &mut Frame, area: Rect, state: &AppState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(area);

    let fields = SettingsField::all();
    let items: Vec<ListItem> = fields.iter().map(|f| {
        ListItem::new(format!("  {}: {}", f.label(), state.settings.field_value(f)))
    }).collect();

    let list = List::new(items)
        .block(Block::default().title(" Settings ").borders(Borders::ALL))
        .highlight_style(Style::default().bg(Color::Blue).fg(Color::White))
        .highlight_symbol(">> ");

    let mut list_state = ListState::default();
    list_state.select(Some(state.settings.selected_field));
    frame.render_stateful_widget(list, chunks[0], &mut list_state);

    let hints = Paragraph::new(" [↑↓] Navigate  [Enter] Select  [Tab] Back to Library")
        .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(hints, chunks[1]);
}

fn draw_cookie_list(frame: &mut Frame, area: Rect, state: &AppState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(area);

    let items: Vec<ListItem> = state.settings.cookie_stores.iter().map(|cs| {
        ListItem::new(format!("  {}: {}", cs.domain, cs.preview()))
    }).collect();

    let list = List::new(items)
        .block(Block::default().title(" Cookies ").borders(Borders::ALL))
        .highlight_style(Style::default().bg(Color::Blue).fg(Color::White))
        .highlight_symbol(">> ");

    let mut list_state = ListState::default();
    list_state.select(Some(state.settings.selected_cookie));
    frame.render_stateful_widget(list, chunks[0], &mut list_state);

    let hints = Paragraph::new(" [↑↓] Navigate  [Enter] Edit  [Esc] Back")
        .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(hints, chunks[1]);
}

fn draw_cookie_edit(frame: &mut Frame, area: Rect, state: &AppState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(area);

    let domain = state.settings.cookie_stores
        .get(state.settings.selected_cookie)
        .map(|cs| cs.domain.as_str())
        .unwrap_or("unknown");

    let block = Block::default()
        .title(format!(" Editing cookies for: {} ", domain))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));

    let paragraph = Paragraph::new(state.settings.cookie_edit_buffer.as_str())
        .block(block)
        .wrap(ratatui::widgets::Wrap { trim: false });

    frame.render_widget(paragraph, chunks[0]);

    let hints = Paragraph::new(" [Enter] Save  [Esc] Cancel")
        .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(hints, chunks[1]);
}
