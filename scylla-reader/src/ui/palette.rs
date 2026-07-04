use crate::messenger::AppCommand;
use crate::state::modal::PaletteAction;
use crate::state::{Modal, Page};
use std::sync::mpsc;

pub fn build_palette_actions(_cmd_tx: mpsc::Sender<AppCommand>) -> Vec<PaletteAction> {
    vec![
        // Navigation (always shown)
        PaletteAction {
            category: "Navigation",
            label: "Go to Library",
            keys: "1",
            handler: |s, _| s.current_page = Page::Library,
        },
        PaletteAction {
            category: "Navigation",
            label: "Go to Reader",
            keys: "2",
            handler: |s, _| {
                s.current_page = Page::Reader;
            },
        },
        PaletteAction {
            category: "Navigation",
            label: "Go to Settings",
            keys: "3",
            handler: |s, _| s.current_page = Page::Settings,
        },
        PaletteAction {
            category: "Navigation",
            label: "Quit",
            keys: "q",
            handler: |_s, _| { /* handled in app loop */ },
        },
        // Library actions
        PaletteAction {
            category: "Library",
            label: "Add Book",
            keys: "i",
            handler: |s, _| {
                s.modal = Modal::AddBook {
                    inputs: vec![String::new()],
                    cursor: 0,
                    scroll_offset: 0,
                };
                s.current_page = Page::AddingBook;
            },
        },
        PaletteAction {
            category: "Library",
            label: "Jump Chapter",
            keys: "j",
            handler: |s, _| {
                if let Some(b) = s.library.selected_book() {
                    s.modal = Modal::JumpChapter {
                        chapters: b.chapters.clone(),
                        cursor: 0,
                        scroll_offset: 0,
                        show_titles: true,
                    };
                    s.current_page = Page::BookChapterJump;
                }
            },
        },
        PaletteAction {
            category: "Library",
            label: "Update All",
            keys: "u",
            handler: |s, tx| {
                let urls: Vec<String> = s
                    .library
                    .books
                    .iter()
                    .map(|b| b.url.clone())
                    .filter(|u| !u.is_empty())
                    .collect();
                if !urls.is_empty() {
                    let _ = tx.send(AppCommand::UpdateAll(urls));
                }
            },
        },
        PaletteAction {
            category: "Library",
            label: "Delete Book",
            keys: "d",
            handler: |s, _| {
                s.library.remove_selected();
            },
        },
        PaletteAction {
            category: "Library",
            label: "Cycle Filter",
            keys: "f",
            handler: |s, _| {
                s.library.cycle_filter();
            },
        },
        PaletteAction {
            category: "Library",
            label: "Cycle Status",
            keys: "Space",
            handler: |s, _| {
                s.library.cycle_selected_status();
            },
        },
        // Reader actions
        PaletteAction {
            category: "Reader",
            label: "Next Chapter",
            keys: ">",
            handler: |s, tx| {
                if let Some(b) = s.library.selected_book() {
                    let next = s.reader.current_chapter_idx + 1;
                    if let Some(ch) = b.chapters.get(next) {
                        s.reader.loading = true;
                        let _ = tx.send(AppCommand::FetchChapter(ch.url.clone(), next));
                    }
                }
            },
        },
        PaletteAction {
            category: "Reader",
            label: "Previous Chapter",
            keys: "<",
            handler: |s, tx| {
                if let Some(b) = s.library.selected_book() {
                    let prev = s.reader.current_chapter_idx.saturating_sub(1);
                    if prev != s.reader.current_chapter_idx {
                        if let Some(ch) = b.chapters.get(prev) {
                            s.reader.loading = true;
                            let _ = tx.send(AppCommand::FetchChapter(ch.url.clone(), prev));
                        }
                    }
                }
            },
        },
        PaletteAction {
            category: "Reader",
            label: "Toggle Paged/Scrollable",
            keys: "",
            handler: |s, _| {
                s.settings.reader_mode = s.settings.reader_mode.toggle();
            },
        },
        // Settings
        PaletteAction {
            category: "Settings",
            label: "Open Settings",
            keys: "3",
            handler: |s, _| s.current_page = Page::Settings,
        },
        PaletteAction {
            category: "Settings",
            label: "Toggle Debug Log",
            keys: "",
            handler: |s, _| {
                s.settings.debug_log = !s.settings.debug_log;
                crate::settings::set_debug(s.settings.debug_log);
            },
        },
        PaletteAction {
            category: "Settings",
            label: "Plugin Configs",
            keys: "",
            handler: |s, _| {
                s.settings.reload_plugins();
                s.settings.settings_page = crate::settings::SettingsPage::PluginList;
            },
        },
        // Debug
        PaletteAction {
            category: "Debug",
            label: "Reload Log",
            keys: "",
            handler: |s, _| s.settings.reload_log(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn test_build_palette_actions_returns_non_empty() {
        let (tx, _rx) = mpsc::channel();
        let actions = build_palette_actions(tx);
        assert!(!actions.is_empty());
        assert!(actions.iter().any(|a| a.label == "Go to Library"));
        assert!(actions.iter().any(|a| a.label == "Add Book"));
        assert!(actions.iter().any(|a| a.label == "Quit"));
    }

    #[test]
    fn test_filter_actions_empty_query_returns_all() {
        let (tx, _rx) = mpsc::channel();
        let actions = build_palette_actions(tx);
        let filtered = filter_actions(&actions, "");
        assert_eq!(filtered.len(), actions.len());
    }

    #[test]
    fn test_filter_actions_fuzzy_matches() {
        let (tx, _rx) = mpsc::channel();
        let actions = build_palette_actions(tx);
        let filtered = filter_actions(&actions, "lib");
        assert!(filtered.iter().any(|a| a.label.to_lowercase().contains("lib")));
    }
}
