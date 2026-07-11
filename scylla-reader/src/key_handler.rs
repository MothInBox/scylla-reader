use crate::input;
use crate::input::keybinds::*;
use crate::messenger::AppCommand;
use crate::settings::SettingsPage;
use crate::state::{AppState, Modal, Page};
use crossterm::event::KeyEvent;
use ratatui::prelude::Rect;
use std::sync::mpsc;

pub fn handle_key(
    state: &mut AppState,
    key: KeyEvent,
    cmd_tx: &mpsc::Sender<AppCommand>,
    size: Rect,
) -> bool {
    if state.ui.modal != Modal::None {
        if key.code == KEY_ESCAPE {
            state.close_modal();
            if matches!(
                state.ui.page,
                Page::AddingBook | Page::BookChapterJump | Page::InstallingPlugin
            ) {
                state.ui.page = Page::Library;
            }
            return true;
        }
        return input::handle_input(state, key, cmd_tx, size);
    }

    match key.code {
        KEY_LIBRARY => {
            crate::settings::log(
                crate::settings::LogLevel::Debug,
                "NAV",
                "Page: Library",
            );
            state.ui.page = Page::Library;
            return true;
        }
        KEY_READER => {
            crate::settings::log(
                crate::settings::LogLevel::Debug,
                "NAV",
                "Page: Reader",
            );
            state.ui.page = Page::Reader;
            if let Some(book) = state.lib.library.selected_book()
                && state.reader.book_url != book.url
            {
                let session = book
                    .active_session_id
                    .and_then(|id| book.sessions.iter().find(|s| s.id == id))
                    .or_else(|| book.sessions.first());
                if let Some(session) = session {
                    state.reader.session_id = session.id;
                    state.reader.session_name = session.name.clone();
                    let idx = (session.progress.current as usize)
                        .min(book.chapters.len().saturating_sub(1));
                    if let Some(ch) = book.chapters.get(idx) {
                        state.reader.loading = true;
                        let _ = cmd_tx.send(AppCommand::FetchChapter(ch.url.clone(), idx));
                    }
                }
            }
            return true;
        }
        KEY_JOBS => {
            crate::settings::log(
                crate::settings::LogLevel::Debug,
                "NAV",
                "Page: Jobs",
            );
            state.ui.page = Page::Jobs;
            return true;
        }
        KEY_SETTINGS => {
            crate::settings::log(
                crate::settings::LogLevel::Debug,
                "NAV",
                "Page: Settings",
            );
            state.ui.page = Page::Settings;
            return true;
        }
        KEY_COMMAND_PALETTE => {
            let actions = crate::ui::palette::build_palette_actions(cmd_tx.clone());
            let filtered = crate::ui::palette::filter_actions(&actions, "");
            state.ui.modal = Modal::CommandPalette {
                query: String::new(),
                filtered,
                selected: 0,
            };
            return true;
        }
        KEY_TOGGLE_HINTS => {
            state.ui.show_hints = !state.ui.show_hints;
            return true;
        }
        KEY_ESCAPE => {
            if state.ui.page == Page::Settings {
                match state.lib.settings_ui.settings_page {
                    SettingsPage::Main => return false,
                    SettingsPage::DebugLog => {
                        state.lib.settings_ui.settings_page = SettingsPage::Main;
                    }
                    SettingsPage::PluginList => {
                        state.lib.settings_ui.settings_page = SettingsPage::Main;
                    }
                    SettingsPage::PluginFields => {
                        state.lib.settings_ui.settings_page = SettingsPage::PluginList;
                    }
                    SettingsPage::PluginFieldEdit => {
                        state.lib.settings_ui.plugin_field_buffer.clear();
                        state.lib.settings_ui.plugin_field_editing = false;
                        state.lib.settings_ui.settings_page = SettingsPage::PluginFields;
                    }
                }
                return true;
            }
            return false;
        }
        _ => {}
    }

    let pre_status = state
        .lib.library
        .selected_book()
        .map(|b| (b.url.clone(), b.status.clone()));
    let pre_books_len = state.lib.library.books.len();
    let removed_url = if key.code == KEY_DELETE {
        state.lib.library.selected_book().map(|b| b.url.clone())
    } else {
        None
    };

    if !input::handle_input(state, key, cmd_tx, size) {
        return false;
    }

    if let Some(url) = removed_url
        && state.lib.library.books.len() < pre_books_len
    {
        state.db.delete_book(&url).unwrap_or_else(|e| {
            crate::settings::log(
                crate::settings::LogLevel::Debug,
                "UI",
                &format!("DB delete failed: {}", e),
            );
        });
    }

    if let Some((url, old_status)) = pre_status
        && let Some(book) = state.lib.library.books.iter().find(|b| b.url == url)
        && book.status != old_status
    {
        state
            .db
            .update_status(&book.url, &book.status)
            .unwrap_or_else(|e| {
                crate::settings::log(
                    crate::settings::LogLevel::Debug,
                    "UI",
                    &format!("DB status update failed: {}", e),
                );
            });
    }

    true
}
