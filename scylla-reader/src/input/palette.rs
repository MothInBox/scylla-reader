//! Palette input handler.

use crate::messenger::AppCommand;
use crate::state::Modal;
use crate::state::AppState;
use crossterm::event::{KeyCode, KeyEvent};
use std::sync::mpsc;

pub fn handle_palette(state: &mut AppState, key: KeyEvent, cmd_tx: &mpsc::Sender<AppCommand>) -> bool {
    if let Modal::CommandPalette { query, filtered, selected } = &mut state.modal {
        match key.code {
            KeyCode::Esc => {
                state.close_modal();
                true
            }
            KeyCode::Enter => {
                if let Some(action) = filtered.get(*selected) {
                    (action.handler)(state, cmd_tx);
                }
                state.close_modal();
                true
            }
            KeyCode::Up => {
                *selected = selected.saturating_sub(1);
                true
            }
            KeyCode::Down => {
                if *selected + 1 < filtered.len() {
                    *selected += 1;
                }
                true
            }
            KeyCode::Char(c) => {
                query.push(c);
                *filtered = crate::ui::palette::filter_actions(
                    &crate::ui::palette::build_palette_actions(cmd_tx.clone()),
                    query,
                );
                *selected = 0;
                true
            }
            KeyCode::Backspace => {
                query.pop();
                *filtered = crate::ui::palette::filter_actions(
                    &crate::ui::palette::build_palette_actions(cmd_tx.clone()),
                    query,
                );
                *selected = 0;
                true
            }
            _ => true,
        }
    } else {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;
    use crate::library::Library;
    use crate::state::Page;
    use crossterm::event::KeyModifiers;

    fn test_state() -> AppState {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let db = Db::open_conn(conn).unwrap();
        AppState::from_parts(db, Library::new())
    }

    fn key_event(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn channel() -> (mpsc::Sender<AppCommand>, mpsc::Receiver<AppCommand>) {
        mpsc::channel()
    }

    fn setup_palette_state(query: &str, selected: usize) -> AppState {
        let (tx, _) = channel();
        let actions = crate::ui::palette::build_palette_actions(tx);
        let filtered = crate::ui::palette::filter_actions(&actions, query);
        let mut state = test_state();
        state.modal = Modal::CommandPalette {
            query: query.to_string(),
            filtered,
            selected,
        };
        state.current_page = Page::Library;
        state
    }

    #[test]
    fn test_handle_palette_esc_closes() {
        let mut state = setup_palette_state("", 0);
        let (tx, _rx) = channel();
        let result = handle_palette(&mut state, key_event(KeyCode::Esc), &tx);
        assert!(result);
        assert_eq!(state.modal, Modal::None);
    }

    #[test]
    fn test_handle_palette_enter_executes_handler_and_closes() {
        let (tx, _rx) = channel();
        let mut state = setup_palette_state("Settings", 0);
        let has_settings = match &state.modal {
            Modal::CommandPalette { filtered, .. } => !filtered.is_empty(),
            _ => false,
        };
        assert!(has_settings, "Expected at least 'Go to Settings' to match 'Settings'");
        handle_palette(&mut state, key_event(KeyCode::Enter), &tx);
        assert_eq!(state.current_page, Page::Settings);
        assert_eq!(state.modal, Modal::None);
    }

    #[test]
    fn test_handle_palette_up_down_navigates() {
        let mut state = setup_palette_state("", 0);
        let (tx, _rx) = channel();
        // Initially selected = 0, press Down -> selected = 1
        handle_palette(&mut state, key_event(KeyCode::Down), &tx);
        if let Modal::CommandPalette { selected, .. } = &state.modal {
            assert_eq!(*selected, 1);
        } else {
            panic!("Expected CommandPalette");
        }
        // Press Up -> selected = 0
        handle_palette(&mut state, key_event(KeyCode::Up), &tx);
        if let Modal::CommandPalette { selected, .. } = &state.modal {
            assert_eq!(*selected, 0);
        } else {
            panic!("Expected CommandPalette");
        }
    }

    #[test]
    fn test_handle_palette_up_stays_at_top() {
        let mut state = setup_palette_state("", 0);
        let (tx, _rx) = channel();
        handle_palette(&mut state, key_event(KeyCode::Up), &tx);
        if let Modal::CommandPalette { selected, .. } = &state.modal {
            assert_eq!(*selected, 0);
        } else {
            panic!("Expected CommandPalette");
        }
    }

    #[test]
    fn test_handle_palette_down_stays_at_bottom() {
        let mut state = setup_palette_state("", 0);
        let (tx, _rx) = channel();
        // Get the filtered list length
        let action_count = if let Modal::CommandPalette { filtered, .. } = &state.modal {
            filtered.len()
        } else {
            0
        };
        // Set selected to last item
        if let Modal::CommandPalette { selected, .. } = &mut state.modal {
            *selected = action_count.saturating_sub(1);
        }
        handle_palette(&mut state, key_event(KeyCode::Down), &tx);
        if let Modal::CommandPalette { selected, .. } = &state.modal {
            assert_eq!(*selected, action_count.saturating_sub(1));
        } else {
            panic!("Expected CommandPalette");
        }
    }

    #[test]
    fn test_handle_palette_type_filters() {
        let mut state = setup_palette_state("", 0);
        let (tx, _rx) = channel();
        // Before typing, all actions are visible
        let initial_count = if let Modal::CommandPalette { filtered, .. } = &state.modal {
            filtered.len()
        } else {
            0
        };
        assert!(initial_count > 0);

        handle_palette(&mut state, key_event(KeyCode::Char('S')), &tx);
        if let Modal::CommandPalette { query, filtered, selected } = &state.modal {
            assert_eq!(query, "S");
            assert_eq!(*selected, 0);
            // Only Settings-related items should match "S"
            assert!(filtered.iter().all(|a| {
                a.label.contains('S')
                    || a.label.contains('s')
                    || a.category.contains('S')
                    || a.category.contains('s')
                    || a.keys.contains('S')
                    || a.keys.contains('s')
            }));
            assert!(filtered.len() < initial_count, "Filtered should have fewer items");
        } else {
            panic!("Expected CommandPalette");
        }
    }

    #[test]
    fn test_handle_palette_backspace_removes_char() {
        let mut state = setup_palette_state("Se", 0);
        let (tx, _rx) = channel();
        let count_after_two_chars = if let Modal::CommandPalette { filtered, .. } = &state.modal {
            filtered.len()
        } else {
            0
        };

        handle_palette(&mut state, key_event(KeyCode::Backspace), &tx);
        if let Modal::CommandPalette { query, filtered, selected } = &state.modal {
            assert_eq!(query, "S");
            assert_eq!(*selected, 0);
            assert!(filtered.len() >= count_after_two_chars, "Backspace should broaden results");
        } else {
            panic!("Expected CommandPalette");
        }
    }

    #[test]
    fn test_handle_palette_unrelated_key_does_nothing_returns_true() {
        let mut state = setup_palette_state("", 0);
        let (tx, _rx) = channel();
        let result = handle_palette(&mut state, key_event(KeyCode::F(1)), &tx);
        assert!(result);
        // State should remain unchanged
        if let Modal::CommandPalette { query, filtered, selected } = &state.modal {
            assert_eq!(query, "");
            assert_eq!(*selected, 0);
            assert!(!filtered.is_empty());
        } else {
            panic!("Expected CommandPalette");
        }
    }

    #[test]
    fn test_handle_palette_not_command_palette_returns_true() {
        let mut state = test_state();
        state.modal = Modal::None;
        let (tx, _rx) = channel();
        let result = handle_palette(&mut state, key_event(KeyCode::Enter), &tx);
        assert!(result);
        assert_eq!(state.modal, Modal::None);
    }
}
