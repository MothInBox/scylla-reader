use crate::state::modal::FilterRow;
use crate::state::palette_action::PaletteAction;
use crate::state::{Modal, Page, UiState};
use crate::ui::widgets::centered_rect;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState};

pub fn build_palette_actions() -> Vec<PaletteAction> {
    vec![
        // Navigation (always shown)
        PaletteAction {
            category: "Navigation",
            label: "Go to Library",
            keys: "1", // KEY_LIBRARY
            handler: |s| s.ui.page = Page::Library,
        },
        PaletteAction {
            category: "Navigation",
            label: "Go to Reader",
            keys: "2", // KEY_READER
            handler: |s| {
                s.ui.page = Page::Reader;
            },
        },
        PaletteAction {
            category: "Navigation",
            label: "Go to Settings",
            keys: "3", // KEY_SETTINGS
            handler: |s| s.ui.page = Page::Settings,
        },
        // Library actions
        PaletteAction {
            category: "Library",
            label: "Add Book",
            keys: "i", // KEY_ADD_BOOK
            handler: |s| {
                s.ui.modal = Modal::AddBook {
                    inputs: vec![String::new()],
                    cursor: 0,
                    scroll_offset: 0,
                };
            },
        },
        PaletteAction {
            category: "Library",
            label: "Jump Chapter",
            keys: "j", // KEY_JUMP_CHAPTER
            handler: |s| {
                if let Some(b) = s.lib.library.selected_book() {
                    s.ui.modal = Modal::JumpChapter {
                        chapters: b.chapters.clone(),
                        query: String::new(),
                        filtered: b.chapters.clone(),
                        cursor: 0,
                        scroll_offset: 0,
                        show_titles: true,
                    };
                }
            },
        },
        PaletteAction {
            category: "Library",
            label: "Update All",
            keys: "u", // KEY_UPDATE_ALL
            handler: |s| {
                let urls: Vec<String> = s
                    .lib
                    .library
                    .books
                    .iter()
                    .map(|b| b.url.clone())
                    .filter(|u| !u.is_empty())
                    .collect();
                if !urls.is_empty() {
                    let base = crate::storage::client::api_base(s);
                    for url in urls {
                        if let Err(e) = crate::storage::client::block_on(
                            crate::storage::client::enqueue_job(&base, "Scrape", &url, None),
                        ) {
                            crate::settings::log(
                                crate::settings::LogLevel::Error,
                                "INPUT",
                                &format!("Failed to enqueue scrape: {}", e),
                            );
                        }
                    }
                }
            },
        },
        PaletteAction {
            category: "Library",
            label: "Delete Book",
            keys: "d", // KEY_DELETE
            handler: |s| {
                s.lib.library.remove_selected();
            },
        },
        PaletteAction {
            category: "Library",
            label: "Filter Library…",
            keys: "f", // KEY_FILTER
            handler: |s| {
                s.ui.modal = Modal::Filter {
                    working: s.lib.library.filter.clone(),
                    focus: FilterRow::Search,
                    tag_query: String::new(),
                    tag_cursor: 0,
                    tag_scroll: 0,
                    status_cursor: s.lib.library.filter.status_cursor(),
                    lib_cursor: s
                        .lib
                        .library
                        .filter
                        .library_cursor(&s.lib.manager.backend_names()),
                    ai_query: String::new(),
                };
            },
        },
        PaletteAction {
            category: "Library",
            label: "Cycle Status",
            keys: "Space", // KEY_CYCLE_STATUS
            handler: |s| {
                s.lib.library.cycle_selected_status();
            },
        },
        // Reader actions
        PaletteAction {
            category: "Reader",
            label: "Next Chapter",
            keys: ">", // KEY_NEXT_CHAPTER
            handler: |s| {
                let next = s.reader.current_chapter_idx + 1;
                let chapter = s
                    .lib
                    .library
                    .selected_book()
                    .and_then(|b| b.chapters.get(next).cloned());
                if let Some(ch) = chapter {
                    s.reader.loading = true;
                    let base = crate::storage::client::api_base(s);
                    if let Err(e) =
                        crate::storage::client::block_on(crate::storage::client::enqueue_job(
                            &base,
                            "FetchChapter",
                            &ch.url,
                            Some(next),
                        ))
                    {
                        crate::settings::log(
                            crate::settings::LogLevel::Error,
                            "INPUT",
                            &format!("Failed to enqueue chapter fetch: {}", e),
                        );
                        s.reader.loading = false;
                    }
                }
            },
        },
        PaletteAction {
            category: "Reader",
            label: "Previous Chapter",
            keys: "<", // KEY_PREV_CHAPTER
            handler: |s| {
                let prev = s.reader.current_chapter_idx.saturating_sub(1);
                let chapter = if prev != s.reader.current_chapter_idx {
                    s.lib
                        .library
                        .selected_book()
                        .and_then(|b| b.chapters.get(prev).cloned())
                } else {
                    None
                };
                if let Some(ch) = chapter {
                    s.reader.loading = true;
                    let base = crate::storage::client::api_base(s);
                    if let Err(e) =
                        crate::storage::client::block_on(crate::storage::client::enqueue_job(
                            &base,
                            "FetchChapter",
                            &ch.url,
                            Some(prev),
                        ))
                    {
                        crate::settings::log(
                            crate::settings::LogLevel::Error,
                            "INPUT",
                            &format!("Failed to enqueue chapter fetch: {}", e),
                        );
                        s.reader.loading = false;
                    }
                }
            },
        },
        PaletteAction {
            category: "Reader",
            label: "Toggle Paged/Scrollable",
            keys: "",
            handler: |s| {
                s.lib.settings.reader_mode = s.lib.settings.reader_mode.toggle();
            },
        },
        // Settings
        PaletteAction {
            category: "Settings",
            label: "Open Settings",
            keys: "3", // KEY_SETTINGS
            handler: |s| s.ui.page = Page::Settings,
        },
        PaletteAction {
            category: "Settings",
            label: "Toggle Debug Log",
            keys: "",
            handler: |s| {
                s.lib.settings.debug_log = !s.lib.settings.debug_log;
                crate::settings::set_debug(s.lib.settings.debug_log);
            },
        },
        PaletteAction {
            category: "Settings",
            label: "Plugin Configs",
            keys: "",
            handler: |s| {
                s.lib.settings.reload_plugins();
                s.lib.settings_ui.settings_page = crate::settings::SettingsPage::PluginList;
            },
        },
        // Plugins
        PaletteAction {
            category: "Plugins",
            label: "Install Plugin from GitHub",
            keys: "",
            handler: |s| {
                s.ui.modal = Modal::InstallPlugin {
                    url: String::new(),
                    cursor: 0,
                    scroll_offset: 0,
                };
            },
        },
        // Debug
        PaletteAction {
            category: "Debug",
            label: "Reload Log",
            keys: "",
            handler: |s| s.lib.settings_ui.reload_log(),
        },
    ]
}

pub fn filter_actions(actions: &[PaletteAction], query: &str) -> Vec<PaletteAction> {
    if query.trim().is_empty() {
        return actions.to_vec();
    }
    let q = query.to_lowercase();
    actions
        .iter()
        .filter(|a| {
            a.label.to_lowercase().contains(&q)
                || a.category.to_lowercase().contains(&q)
                || a.keys.to_lowercase().contains(&q)
        })
        .cloned()
        .collect()
}

pub fn draw_palette(frame: &mut Frame, area: Rect, ui: &UiState) {
    if let Modal::CommandPalette {
        query,
        filtered,
        selected,
    } = &ui.modal
    {
        let popup_area = centered_rect(60, 40, area);
        frame.render_widget(Clear, popup_area);

        let items: Vec<ListItem> = filtered
            .iter()
            .map(|a| {
                let keys = if a.keys.is_empty() {
                    String::new()
                } else {
                    format!(" [{}]", a.keys)
                };
                ListItem::new(format!("{}  {}{}", a.category, a.label, keys))
            })
            .collect();

        let mut list_state = ListState::default();
        list_state.select(Some(*selected));

        let list = List::new(items)
            .block(
                Block::default()
                    .title(format!(" Commands ({}): ", query))
                    .borders(Borders::ALL),
            )
            .highlight_style(Style::default().bg(Color::Blue).fg(Color::White))
            .highlight_symbol(">> ");

        frame.render_stateful_widget(list, popup_area, &mut list_state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::Library;
    use crate::state::AppState;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    #[test]
    fn test_build_palette_actions_returns_non_empty() {
        let actions = build_palette_actions();
        assert!(!actions.is_empty());
        assert!(actions.iter().any(|a| a.label == "Go to Library"));
        assert!(actions.iter().any(|a| a.label == "Add Book"));
    }

    #[test]
    fn test_filter_actions_empty_query_returns_all() {
        let actions = build_palette_actions();
        let filtered = filter_actions(&actions, "");
        assert_eq!(filtered.len(), actions.len());
    }

    #[test]
    fn test_filter_actions_fuzzy_matches() {
        let actions = build_palette_actions();
        let filtered = filter_actions(&actions, "lib");
        assert!(
            filtered
                .iter()
                .any(|a| a.label.to_lowercase().contains("lib"))
        );
    }

    #[test]
    fn test_draw_palette_renders_without_panic() {
        let mut state = AppState::from_parts(Library::new());
        state.ui.modal = Modal::CommandPalette {
            query: "test".into(),
            filtered: vec![],
            selected: 0,
        };

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                draw_palette(f, f.area(), &state.ui);
            })
            .unwrap();
    }
}
