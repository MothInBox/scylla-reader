use crate::settings::{SettingsField, SettingsPage};
use crate::state::{LibraryState, UiState};
use crate::ui::widgets::hint_line;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use std::net::TcpStream;
use std::time::Duration;

/// Cursor shown after the edit buffer while editing a field value.
const EDIT_CURSOR: &str = "\u{2588}"; // █

/// Extract the `SocketAddr` from a backend URL like `http://127.0.0.1:8080`.
fn socket_addr_from_url(url: &str) -> Option<std::net::SocketAddr> {
    let without_scheme = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))
        .unwrap_or(url);
    let host_port = without_scheme.split('/').next().unwrap_or(without_scheme);
    host_port.parse().ok()
}

pub fn draw(frame: &mut Frame, area: Rect, lib: &LibraryState, ui: &UiState) {
    match lib.settings_ui.settings_page {
        SettingsPage::Main => draw_main(frame, area, lib, ui),
        SettingsPage::DebugLog => draw_debug_log(frame, area, lib, ui),
        SettingsPage::PluginList => draw_plugin_list(frame, area, lib, ui),
        SettingsPage::PluginFields => draw_plugin_fields(frame, area, lib, ui),
        SettingsPage::PluginFieldEdit => draw_plugin_field_edit(frame, area, lib, ui),
        SettingsPage::Server => draw_server_page(frame, area, lib, ui),
    }
}

fn draw_main(frame: &mut Frame, area: Rect, lib: &LibraryState, ui: &UiState) {
    let hint_height: u16 = if ui.show_hints { 2 } else { 1 };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(hint_height)])
        .split(area);

    let fields = SettingsField::all();
    let items: Vec<ListItem> = fields
        .iter()
        .map(|f| {
            let value = if *f == SettingsField::RateLimit && lib.settings_ui.editing {
                format!("{}{}", lib.settings_ui.edit_buffer, EDIT_CURSOR)
            } else if *f == SettingsField::AutoEmbed {
                if lib.server_settings.autoembed {
                    "ON".to_string()
                } else {
                    "OFF".to_string()
                }
            } else {
                lib.settings.field_value(f)
            };
            ListItem::new(format!("  {}: {}", f.label(), value))
        })
        .collect();

    let list = List::new(items)
        .block(Block::default().title(" Settings ").borders(Borders::ALL))
        .highlight_style(Style::default().bg(Color::Blue).fg(Color::White))
        .highlight_symbol(">> ");

    let mut list_state = ListState::default();
    list_state.select(Some(lib.settings_ui.selected_field));
    frame.render_stateful_widget(list, chunks[0], &mut list_state);

    draw_hints(frame, chunks[1], ui.show_hints);
}

fn draw_debug_log(frame: &mut Frame, area: Rect, lib: &LibraryState, ui: &UiState) {
    let hint_height: u16 = if ui.show_hints { 2 } else { 1 };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(hint_height),
        ])
        .split(area);

    let toggle_status = if lib.settings.debug_log { "ON" } else { "OFF" };
    let toggle_line = Paragraph::new(format!(
        " Debug Logging: {}    [Enter] Toggle",
        toggle_status,
    ))
    .block(Block::default().title(" Debug Log ").borders(Borders::ALL));
    frame.render_widget(toggle_line, chunks[0]);

    let log_content = lib.settings_ui.log_lines.join("\n");
    let log_widget = Paragraph::new(log_content)
        .block(Block::default().title(" Log ").borders(Borders::ALL))
        .scroll((lib.settings_ui.log_scroll as u16, 0))
        .wrap(ratatui::widgets::Wrap { trim: false });
    frame.render_widget(log_widget, chunks[1]);

    draw_hints(frame, chunks[2], ui.show_hints);
}

fn draw_plugin_list(frame: &mut Frame, area: Rect, lib: &LibraryState, ui: &UiState) {
    let hint_height: u16 = if ui.show_hints { 2 } else { 1 };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(hint_height)])
        .split(area);

    let items: Vec<ListItem> = lib
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
    list_state.select(Some(lib.settings_ui.selected_plugin));
    frame.render_stateful_widget(list, chunks[0], &mut list_state);

    draw_hints(frame, chunks[1], ui.show_hints);
}

fn draw_plugin_fields(frame: &mut Frame, area: Rect, lib: &LibraryState, ui: &UiState) {
    let hint_height: u16 = if ui.show_hints { 2 } else { 1 };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(hint_height)])
        .split(area);

    let Some(config) = lib
        .settings
        .plugin_configs
        .get(lib.settings_ui.selected_plugin)
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
    let clamped = lib
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

    draw_hints(frame, chunks[1], ui.show_hints);
}

fn draw_plugin_field_edit(frame: &mut Frame, area: Rect, lib: &LibraryState, ui: &UiState) {
    let hint_height: u16 = if ui.show_hints { 2 } else { 1 };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(hint_height)])
        .split(area);

    let config = lib
        .settings
        .plugin_configs
        .get(lib.settings_ui.selected_plugin);
    let is_cookie = config
        .map(|c| lib.settings_ui.selected_plugin_field == c.schema.len() && c.accepts_cookies)
        .unwrap_or(false);

    let title = match (config, is_cookie) {
        (Some(c), true) => format!(" {} \u{2014} Cookies ", c.domain),
        (Some(c), false) => {
            let label = c
                .schema
                .get(lib.settings_ui.selected_plugin_field)
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

    let paragraph = Paragraph::new(lib.settings_ui.plugin_field_buffer.as_str())
        .block(block)
        .wrap(ratatui::widgets::Wrap { trim: false });

    frame.render_widget(paragraph, chunks[0]);

    draw_hints(frame, chunks[1], ui.show_hints);
}

fn draw_server_page(frame: &mut Frame, area: Rect, lib: &LibraryState, ui: &UiState) {
    let hint_height: u16 = if ui.show_hints { 2 } else { 1 };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(hint_height)])
        .split(area);

    let mut connected = lib.server_settings.connected;
    if !connected {
        let probe_addr = lib
            .manager
            .primary_backend()
            .and_then(|b| b.url())
            .and_then(|u| socket_addr_from_url(&u))
            .unwrap_or_else(|| "127.0.0.1:8080".parse().unwrap());
        connected = TcpStream::connect_timeout(&probe_addr, Duration::from_millis(500)).is_ok();
    }

    let status = if connected {
        "Connected"
    } else {
        "Disconnected"
    };
    let status_style = if connected {
        Style::default().fg(Color::Green)
    } else {
        Style::default().fg(Color::Red)
    };

    let mut lines = vec![
        Line::from(vec![
            Span::raw("Status: "),
            Span::styled(status, status_style),
        ]),
        Line::from(format!("Port: {}", lib.server_settings.port)),
        Line::from(format!("Max Workers: {}", lib.server_settings.max_workers)),
        Line::from(format!("Rate Limit: {}s", lib.server_settings.rate_limit)),
    ];

    if !lib.server_settings.plugins.is_empty() {
        lines.push(Line::from("Plugins:"));
        for plugin in &lib.server_settings.plugins {
            lines.push(Line::from(format!("  - {}", plugin)));
        }
    }

    if let Some(ref err) = lib.server_settings.error {
        lines.push(Line::from(Span::styled(
            format!("Error: {}", err),
            Style::default().fg(Color::Red),
        )));
    }

    if lib.server_settings.loading {
        lines.push(Line::from("Loading..."));
    }

    let paragraph = Paragraph::new(lines).block(
        Block::default()
            .title(" Server Connection ")
            .borders(Borders::ALL),
    );
    frame.render_widget(paragraph, chunks[0]);

    if ui.show_hints {
        let hint_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Length(1)])
            .split(chunks[1]);
        frame.render_widget(
            Paragraph::new(hint_line("Actions", &[("s", "Refresh")])),
            hint_chunks[0],
        );
        frame.render_widget(
            Paragraph::new(hint_line("Nav", crate::ui::widgets::NAV_HINTS)),
            hint_chunks[1],
        );
    }
}

fn draw_hints(frame: &mut Frame, area: Rect, show_hints: bool) {
    if !show_hints {
        return;
    }
    let hint_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(area);
    frame.render_widget(
        Paragraph::new(hint_line(
            "Actions",
            &[("\u{2191}/\u{2193}", "Navigate"), ("Enter", "Select")],
        )),
        hint_chunks[0],
    );
    frame.render_widget(
        Paragraph::new(hint_line("Nav", crate::ui::widgets::NAV_HINTS)),
        hint_chunks[1],
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::Library;
    use crate::state::AppState;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn make_state() -> AppState {
        AppState::from_parts(Library::new())
    }

    #[test]
    fn test_settings_draw_shows_hints_when_enabled() {
        let mut state = make_state();
        state.ui.show_hints = true;

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                draw(f, f.area(), &state.lib, &state.ui);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        let content: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(content.contains("[Enter]"));
    }

    #[test]
    fn test_settings_draw_shows_rate_limit_label() {
        let mut state = make_state();
        state.ui.show_hints = true;

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                draw(f, f.area(), &state.lib, &state.ui);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        let content: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(content.contains("Rate Limit"));
    }

    #[test]
    fn test_settings_draw_hides_hints_when_disabled() {
        let mut state = make_state();
        state.ui.show_hints = false;

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                draw(f, f.area(), &state.lib, &state.ui);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        let content: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(!content.contains("[Enter]"));
    }

    #[test]
    fn test_settings_draw_shows_autoembed_value() {
        let mut state = make_state();
        state.lib.server_settings.autoembed = true;

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                draw(f, f.area(), &state.lib, &state.ui);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        let content: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(
            content.contains("Auto-Embed New Chapters"),
            "content: {}",
            content
        );
        assert!(content.contains(": ON"), "content: {}", content);
    }

    #[test]
    fn test_settings_draw_shows_edit_buffer_while_editing_rate_limit() {
        let mut state = make_state();
        state.lib.settings_ui.selected_field = 0; // SettingsField::RateLimit
        state.lib.settings_ui.editing = true;
        state.lib.settings_ui.edit_buffer = "5".to_string();

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                draw(f, f.area(), &state.lib, &state.ui);
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        let content: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(
            content.contains("Rate Limit (seconds between requests): 5\u{2588}"),
            "content: {}",
            content
        );
        assert!(
            !content.contains("Rate Limit (seconds between requests): 2"),
            "content: {}",
            content
        );
    }
}
