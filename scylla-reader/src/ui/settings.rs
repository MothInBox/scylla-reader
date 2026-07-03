use crate::settings::{SettingsField, SettingsPage};
use crate::state::AppState;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};

pub fn draw(frame: &mut Frame, area: Rect, state: &AppState) {
    match state.settings.settings_page {
        SettingsPage::Main => draw_main(frame, area, state),
        SettingsPage::PluginList => draw_plugin_list(frame, area, state),
        SettingsPage::PluginFields => draw_plugin_fields(frame, area, state),
        SettingsPage::PluginFieldEdit => draw_plugin_field_edit(frame, area, state),
    }
}

fn draw_main(frame: &mut Frame, area: Rect, state: &AppState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(area);

    let fields = SettingsField::all();
    let items: Vec<ListItem> = fields
        .iter()
        .map(|f| {
            ListItem::new(format!(
                "  {}: {}",
                f.label(),
                state.settings.field_value(f)
            ))
        })
        .collect();

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

fn draw_plugin_list(frame: &mut Frame, area: Rect, state: &AppState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(area);

    let items: Vec<ListItem> = state
        .settings
        .plugin_configs
        .iter()
        .map(|pc| ListItem::new(format!("  {}: {}", pc.domain, pc.preview())))
        .collect();

    let list = List::new(items)
        .block(Block::default().title(" Plugin Configs ").borders(Borders::ALL))
        .highlight_style(Style::default().bg(Color::Blue).fg(Color::White))
        .highlight_symbol(">> ");

    let mut list_state = ListState::default();
    list_state.select(Some(state.settings.selected_plugin));
    frame.render_stateful_widget(list, chunks[0], &mut list_state);

    let hints = Paragraph::new(" [↑↓] Navigate  [Enter] Select  [Esc] Back")
        .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(hints, chunks[1]);
}

fn draw_plugin_fields(frame: &mut Frame, area: Rect, state: &AppState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(area);

    let Some(config) = state.settings.plugin_configs.get(state.settings.selected_plugin) else {
        return;
    };

    let mut items: Vec<ListItem> = config
        .schema
        .iter()
        .map(|f| {
            let value = config.values.get(&f.key).map(|v| v.as_str()).unwrap_or("");
            ListItem::new(format!("  {}: {}", f.label, value))
        })
        .collect();

    if config.accepts_cookies {
        items.push(ListItem::new(format!(
            "  Cookies: {}",
            config.cookies_preview()
        )));
    }

    let cookie_extra = if config.accepts_cookies { 1 } else { 0 };
    let field_count = config.schema.len() + cookie_extra;
    let clamped = state.settings.selected_plugin_field.min(field_count.saturating_sub(1));
    let list = List::new(items)
        .block(Block::default()
            .title(format!(" {} ", config.domain))
            .borders(Borders::ALL))
        .highlight_style(Style::default().bg(Color::Blue).fg(Color::White))
        .highlight_symbol(">> ");

    let mut list_state = ListState::default();
    list_state.select(Some(clamped));
    frame.render_stateful_widget(list, chunks[0], &mut list_state);

    let hints = Paragraph::new(" [↑↓] Navigate  [Enter] Edit  [Esc] Back")
        .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(hints, chunks[1]);
}

fn draw_plugin_field_edit(frame: &mut Frame, area: Rect, state: &AppState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(area);

    let config = state.settings.plugin_configs.get(state.settings.selected_plugin);
    let is_cookie = config
        .map(|c| state.settings.selected_plugin_field == c.schema.len() && c.accepts_cookies)
        .unwrap_or(false);

    let title = match (config, is_cookie) {
        (Some(c), true) => format!(" {} — Cookies ", c.domain),
        (Some(c), false) => {
            let label = c
                .schema
                .get(state.settings.selected_plugin_field)
                .map(|f| f.label.as_str())
                .unwrap_or("unknown");
            format!(" {} — {} ", c.domain, label)
        }
        (None, _) => " unknown ".to_string(),
    };

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));

    let paragraph = Paragraph::new(state.settings.plugin_field_buffer.as_str())
        .block(block)
        .wrap(ratatui::widgets::Wrap { trim: false });

    frame.render_widget(paragraph, chunks[0]);

    let hints =
        Paragraph::new(" [Enter] Save  [Esc] Cancel").style(Style::default().fg(Color::DarkGray));
    frame.render_widget(hints, chunks[1]);
}
