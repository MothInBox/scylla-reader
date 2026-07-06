use crate::settings::{SettingsField, SettingsPage};
use crate::state::AppState;
use crate::ui::widgets::hint_line;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};

pub fn draw(frame: &mut Frame, area: Rect, state: &AppState) {
    match state.settings_ui.settings_page {
        SettingsPage::Main => draw_main(frame, area, state),
        SettingsPage::DebugLog => draw_debug_log(frame, area, state),
        SettingsPage::PluginList => draw_plugin_list(frame, area, state),
        SettingsPage::PluginFields => draw_plugin_fields(frame, area, state),
        SettingsPage::PluginFieldEdit => draw_plugin_field_edit(frame, area, state),
    }
}

fn draw_main(frame: &mut Frame, area: Rect, state: &AppState) {
    let hint_height: u16 = if state.show_hints { 2 } else { 1 };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(hint_height)])
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
    list_state.select(Some(state.settings_ui.selected_field));
    frame.render_stateful_widget(list, chunks[0], &mut list_state);

    if state.show_hints {
        let hint_area = chunks[1];
        let hint_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Length(1)])
            .split(hint_area);
        frame.render_widget(
            Paragraph::new(hint_line(
                "Actions",
                &[("\u{2191}/\u{2193}", "Navigate"), ("Enter", "Select")],
            )),
            hint_chunks[0],
        );
        frame.render_widget(
            Paragraph::new(hint_line(
                "Nav",
                crate::ui::widgets::NAV_HINTS,
            )),
            hint_chunks[1],
        );
    }
}

fn draw_debug_log(frame: &mut Frame, area: Rect, state: &AppState) {
    let hint_height: u16 = if state.show_hints { 2 } else { 1 };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(hint_height),
        ])
        .split(area);

    let toggle_status = if state.settings.debug_log {
        "ON"
    } else {
        "OFF"
    };
    let toggle_line = Paragraph::new(format!(
        " Debug Logging: {}    [Enter] Toggle",
        toggle_status,
    ))
    .block(Block::default().title(" Debug Log ").borders(Borders::ALL));
    frame.render_widget(toggle_line, chunks[0]);

    let log_content = state.settings_ui.log_lines.join("\n");
    let log_widget = Paragraph::new(log_content)
        .block(Block::default().title(" Log ").borders(Borders::ALL))
        .scroll((state.settings_ui.log_scroll as u16, 0))
        .wrap(ratatui::widgets::Wrap { trim: false });
    frame.render_widget(log_widget, chunks[1]);

    if state.show_hints {
        let hint_area = chunks[2];
        let hint_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Length(1)])
            .split(hint_area);
        frame.render_widget(
            Paragraph::new(hint_line(
                "Actions",
                &[("\u{2191}/\u{2193}", "Navigate"), ("Enter", "Select")],
            )),
            hint_chunks[0],
        );
        frame.render_widget(
            Paragraph::new(hint_line(
                "Nav",
                crate::ui::widgets::NAV_HINTS,
            )),
            hint_chunks[1],
        );
    }
}

fn draw_plugin_list(frame: &mut Frame, area: Rect, state: &AppState) {
    let hint_height: u16 = if state.show_hints { 2 } else { 1 };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(hint_height)])
        .split(area);

    let items: Vec<ListItem> = state
        .settings
        .plugin_configs
        .iter()
        .map(|pc| ListItem::new(format!("  {}: {}", pc.domain, pc.preview())))
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .title(" Plugin Configs ")
                .borders(Borders::ALL),
        )
        .highlight_style(Style::default().bg(Color::Blue).fg(Color::White))
        .highlight_symbol(">> ");

    let mut list_state = ListState::default();
    list_state.select(Some(state.settings_ui.selected_plugin));
    frame.render_stateful_widget(list, chunks[0], &mut list_state);

    if state.show_hints {
        let hint_area = chunks[1];
        let hint_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Length(1)])
            .split(hint_area);
        frame.render_widget(
            Paragraph::new(hint_line(
                "Actions",
                &[("\u{2191}/\u{2193}", "Navigate"), ("Enter", "Select")],
            )),
            hint_chunks[0],
        );
        frame.render_widget(
            Paragraph::new(hint_line(
                "Nav",
                crate::ui::widgets::NAV_HINTS,
            )),
            hint_chunks[1],
        );
    }
}

fn draw_plugin_fields(frame: &mut Frame, area: Rect, state: &AppState) {
    let hint_height: u16 = if state.show_hints { 2 } else { 1 };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(hint_height)])
        .split(area);

    let Some(config) = state
        .settings
        .plugin_configs
        .get(state.settings_ui.selected_plugin)
    else {
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
    let clamped = state
        .settings_ui
        .selected_plugin_field
        .min(field_count.saturating_sub(1));
    let list = List::new(items)
        .block(
            Block::default()
                .title(format!(" {} ", config.domain))
                .borders(Borders::ALL),
        )
        .highlight_style(Style::default().bg(Color::Blue).fg(Color::White))
        .highlight_symbol(">> ");

    let mut list_state = ListState::default();
    list_state.select(Some(clamped));
    frame.render_stateful_widget(list, chunks[0], &mut list_state);

    if state.show_hints {
        let hint_area = chunks[1];
        let hint_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Length(1)])
            .split(hint_area);
        frame.render_widget(
            Paragraph::new(hint_line(
                "Actions",
                &[("\u{2191}/\u{2193}", "Navigate"), ("Enter", "Select")],
            )),
            hint_chunks[0],
        );
        frame.render_widget(
            Paragraph::new(hint_line(
                "Nav",
                crate::ui::widgets::NAV_HINTS,
            )),
            hint_chunks[1],
        );
    }
}

fn draw_plugin_field_edit(frame: &mut Frame, area: Rect, state: &AppState) {
    let hint_height: u16 = if state.show_hints { 2 } else { 1 };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(hint_height)])
        .split(area);

    let config = state
        .settings
        .plugin_configs
        .get(state.settings_ui.selected_plugin);
    let is_cookie = config
        .map(|c| state.settings_ui.selected_plugin_field == c.schema.len() && c.accepts_cookies)
        .unwrap_or(false);

    let title = match (config, is_cookie) {
        (Some(c), true) => format!(" {} \u{2014} Cookies ", c.domain),
        (Some(c), false) => {
            let label = c
                .schema
                .get(state.settings_ui.selected_plugin_field)
                .map(|f| f.label.as_str())
                .unwrap_or("unknown");
            format!(" {} \u{2014} {} ", c.domain, label)
        }
        (None, _) => " unknown ".to_string(),
    };

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));

    let paragraph = Paragraph::new(state.settings_ui.plugin_field_buffer.as_str())
        .block(block)
        .wrap(ratatui::widgets::Wrap { trim: false });

    frame.render_widget(paragraph, chunks[0]);

    if state.show_hints {
        let hint_area = chunks[1];
        let hint_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Length(1)])
            .split(hint_area);
        frame.render_widget(
            Paragraph::new(hint_line(
                "Actions",
                &[("\u{2191}/\u{2193}", "Navigate"), ("Enter", "Select")],
            )),
            hint_chunks[0],
        );
        frame.render_widget(
            Paragraph::new(hint_line(
                "Nav",
                crate::ui::widgets::NAV_HINTS,
            )),
            hint_chunks[1],
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;
    use crate::library::Library;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    #[test]
    fn test_settings_draw_shows_hints_when_enabled() {
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
        assert!(content.contains("[Enter]"));
    }

    #[test]
    fn test_settings_draw_shows_rate_limit_label() {
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
        assert!(content.contains("Rate Limit"));
    }

    #[test]
    fn test_settings_draw_hides_hints_when_disabled() {
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
        assert!(!content.contains("[Enter]"));
    }
}