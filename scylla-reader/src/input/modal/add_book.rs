use crate::input::keybinds::*;
use crate::messenger::AppCommand;
use crate::state::{Modal, Page, UiState};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub fn handle_adding_book(
    ui: &mut UiState,
    key: KeyEvent,
    cmd_tx: &std::sync::mpsc::Sender<AppCommand>,
) -> bool {
    if let (KEY_SUBMIT_MODIFIER, KEY_SUBMIT) = (key.modifiers, key.code) {
        let urls: Vec<String> = if let Modal::AddBook { inputs, .. } = &ui.modal {
            inputs
                .iter()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        } else {
            vec![]
        };

        if urls.is_empty() {
            crate::settings::log(
                crate::settings::LogLevel::Debug,
                "INPUT",
                "No valid URLs to scrape",
            );
        } else {
            crate::settings::log(
                crate::settings::LogLevel::Debug,
                "INPUT",
                &format!("Submitting {} URLs", urls.len()),
            );
            for url in urls {
                if let Err(e) = cmd_tx.send(AppCommand::Scrape(url)) {
                    crate::settings::log(
                        crate::settings::LogLevel::Error,
                        "INPUT",
                        &format!("Failed to queue scrape: {}", e),
                    );
                }
            }
        }

        ui.modal = Modal::None;
        ui.page = Page::Library;
        return true;
    }

    if let Modal::AddBook { inputs, cursor, .. } = &mut ui.modal {
        match key.code {
            KEY_ENTER => {
                inputs.insert(*cursor + 1, String::new());
                *cursor += 1;
            }
            KEY_BACKSPACE => {
                let line_empty = inputs[*cursor].is_empty();
                if line_empty && inputs.len() > 1 {
                    inputs.remove(*cursor);
                    if *cursor > 0 {
                        *cursor -= 1;
                    }
                } else {
                    inputs[*cursor].pop();
                }
            }
            KEY_NAV_UP => {
                if *cursor > 0 {
                    *cursor -= 1;
                }
            }
            KEY_NAV_DOWN => {
                if *cursor < inputs.len().saturating_sub(1) {
                    *cursor += 1;
                }
            }
            KeyCode::Char(c)
                if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
            {
                inputs[*cursor].push(c);
            }
            _ => {}
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::*;
    use crossterm::event::KeyCode;

    fn key_event_ctrl_s() -> KeyEvent {
        KeyEvent::new(KEY_SUBMIT, KEY_SUBMIT_MODIFIER)
    }

    fn setup_add_book_state(inputs: Vec<String>, cursor: usize) -> crate::state::AppState {
        let mut state = test_state();
        state.ui.modal = Modal::AddBook {
            inputs,
            cursor,
            scroll_offset: 0,
        };
        state
    }

    #[test]
    fn test_handle_adding_book_ctrl_s_submits() {
        let mut state = setup_add_book_state(
            vec!["http://example.com".into(), "http://test.org".into()],
            0,
        );
        let (tx, rx) = channel();
        let result = handle_adding_book(&mut state.ui, key_event_ctrl_s(), &tx);
        assert!(result);
        assert_eq!(state.ui.modal, Modal::None);
        assert_eq!(state.ui.page, Page::Library);

        let received: Vec<AppCommand> = rx.try_iter().collect();
        assert_eq!(received.len(), 2);
        assert!(matches!(&received[0], AppCommand::Scrape(url) if url == "http://example.com"));
        assert!(matches!(&received[1], AppCommand::Scrape(url) if url == "http://test.org"));
    }

    #[test]
    fn test_handle_adding_book_ctrl_s_empty_inputs_does_not_submit() {
        let mut state = setup_add_book_state(vec!["".into(), "  ".into()], 0);
        let (tx, rx) = channel();
        let result = handle_adding_book(&mut state.ui, key_event_ctrl_s(), &tx);
        assert!(result);
        assert_eq!(state.ui.modal, Modal::None);
        assert_eq!(state.ui.page, Page::Library);

        let received: Vec<AppCommand> = rx.try_iter().collect();
        assert!(received.is_empty());
    }

    #[test]
    fn test_handle_adding_book_enter_adds_new_line() {
        let mut state = setup_add_book_state(vec!["line1".into(), "line2".into()], 0);
        let (tx, _rx) = channel();
        let result = handle_adding_book(&mut state.ui, key_event(KEY_ENTER), &tx);
        assert!(result);
        if let Modal::AddBook { inputs, cursor, .. } = &state.ui.modal {
            assert_eq!(inputs.len(), 3);
            assert_eq!(inputs[0], "line1");
            assert_eq!(inputs[1], "");
            assert_eq!(inputs[2], "line2");
            assert_eq!(*cursor, 1);
        } else {
            panic!("Expected AddBook modal");
        }
    }

    #[test]
    fn test_handle_adding_book_backspace_deletes_empty_line() {
        let mut state = setup_add_book_state(vec!["a".into(), "".into(), "b".into()], 1);
        let (tx, _rx) = channel();
        let result = handle_adding_book(&mut state.ui, key_event(KEY_BACKSPACE), &tx);
        assert!(result);
        if let Modal::AddBook { inputs, cursor, .. } = &state.ui.modal {
            assert_eq!(inputs.len(), 2);
            assert_eq!(inputs[0], "a");
            assert_eq!(inputs[1], "b");
            assert_eq!(*cursor, 0);
        } else {
            panic!("Expected AddBook modal");
        }
    }

    #[test]
    fn test_handle_adding_book_backspace_pops_char_on_non_empty_line() {
        let mut state = setup_add_book_state(vec!["hello".into()], 0);
        let (tx, _rx) = channel();
        let result = handle_adding_book(&mut state.ui, key_event(KEY_BACKSPACE), &tx);
        assert!(result);
        if let Modal::AddBook { inputs, .. } = &state.ui.modal {
            assert_eq!(inputs[0], "hell");
        } else {
            panic!("Expected AddBook modal");
        }
    }

    #[test]
    fn test_handle_adding_book_up_down_navigation() {
        let mut state = setup_add_book_state(vec!["a".into(), "b".into(), "c".into()], 1);
        let (tx, _rx) = channel();
        handle_adding_book(&mut state.ui, key_event(KEY_NAV_UP), &tx);
        if let Modal::AddBook { cursor, .. } = &state.ui.modal {
            assert_eq!(*cursor, 0);
        } else {
            panic!("Expected AddBook modal");
        }
        handle_adding_book(&mut state.ui, key_event(KEY_NAV_DOWN), &tx);
        if let Modal::AddBook { cursor, .. } = &state.ui.modal {
            assert_eq!(*cursor, 1);
        } else {
            panic!("Expected AddBook modal");
        }
    }

    #[test]
    fn test_handle_adding_book_up_stays_at_top() {
        let mut state = setup_add_book_state(vec!["a".into(), "b".into()], 0);
        let (tx, _rx) = channel();
        handle_adding_book(&mut state.ui, key_event(KEY_NAV_UP), &tx);
        if let Modal::AddBook { cursor, .. } = &state.ui.modal {
            assert_eq!(*cursor, 0);
        } else {
            panic!("Expected AddBook modal");
        }
    }

    #[test]
    fn test_handle_adding_book_down_stays_at_bottom() {
        let mut state = setup_add_book_state(vec!["a".into(), "b".into()], 1);
        let (tx, _rx) = channel();
        handle_adding_book(&mut state.ui, key_event(KEY_NAV_DOWN), &tx);
        if let Modal::AddBook { cursor, .. } = &state.ui.modal {
            assert_eq!(*cursor, 1);
        } else {
            panic!("Expected AddBook modal");
        }
    }

    #[test]
    fn test_handle_adding_book_types_characters() {
        let mut state = setup_add_book_state(vec!["he".into()], 0);
        let (tx, _rx) = channel();
        handle_adding_book(&mut state.ui, key_event(KeyCode::Char('l')), &tx);
        handle_adding_book(&mut state.ui, key_event(KeyCode::Char('o')), &tx);
        if let Modal::AddBook { inputs, .. } = &state.ui.modal {
            assert_eq!(inputs[0], "helo");
        } else {
            panic!("Expected AddBook modal");
        }
    }
}
