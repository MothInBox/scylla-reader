use crate::input::keybinds::*;
use crate::messenger::AppCommand;
use crate::state::{UiState, Modal, Page};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub fn handle_installing_plugin(
    ui: &mut UiState,
    key: KeyEvent,
    cmd_tx: &std::sync::mpsc::Sender<AppCommand>,
) -> bool {
    // Submit on Ctrl+S or Enter when URL is non-empty
    if key.code == KEY_ENTER
        || (key.modifiers == KeyModifiers::CONTROL && key.code == KeyCode::Char('s'))
    {
        let url = if let Modal::InstallPlugin { url, .. } = &ui.modal {
            url.trim().to_string()
        } else {
            String::new()
        };

        if !url.is_empty() {
            crate::settings::log(
                crate::settings::LogLevel::Debug,
                "PLUGIN",
                &format!("Installing plugin from: {}", url),
            );
            let _ = cmd_tx.send(AppCommand::InstallPlugin(url));
        }

        ui.modal = Modal::None;
        ui.page = Page::Library;
        return true;
    }

    if let Modal::InstallPlugin {
        url,
        cursor,
        scroll_offset: _,
    } = &mut ui.modal
    {
        match key.code {
            KEY_BACKSPACE => {
                if *cursor > 0 {
                    let before = url[..*cursor - 1].to_string();
                    let after = url[*cursor..].to_string();
                    *url = format!("{}{}", before, after);
                    *cursor -= 1;
                }
            }
            KeyCode::Left => {
                *cursor = cursor.saturating_sub(1);
            }
            KeyCode::Right => {
                if *cursor < url.len() {
                    *cursor += 1;
                }
            }
            KeyCode::Home => {
                *cursor = 0;
            }
            KeyCode::End => {
                *cursor = url.len();
            }
            KeyCode::Delete => {
                if *cursor < url.len() {
                    let before = url[..*cursor].to_string();
                    let after = url[*cursor + 1..].to_string();
                    *url = format!("{}{}", before, after);
                }
            }
            KeyCode::Char(c)
                if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
            {
                let before = url[..*cursor].to_string();
                let after = url[*cursor..].to_string();
                *url = format!("{}{}{}", before, c, after);
                *cursor += 1;
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

    fn setup_install_plugin_state(url: &str, cursor: usize) -> crate::state::AppState {
        let mut state = test_state();
        state.ui.modal = Modal::InstallPlugin {
            url: url.to_string(),
            cursor,
            scroll_offset: 0,
        };
        state.ui.page = Page::InstallingPlugin;
        state
    }

    #[test]
    fn test_handle_installing_plugin_enter_submits() {
        let mut state = setup_install_plugin_state("https://github.com/owner/repo", 0);
        let (tx, rx) = channel();
        let result = handle_installing_plugin(&mut state.ui, key_event(KEY_ENTER), &tx);
        assert!(result);
        assert_eq!(state.ui.modal, Modal::None);
        assert_eq!(state.ui.page, Page::Library);

        let received: Vec<AppCommand> = rx.try_iter().collect();
        assert_eq!(received.len(), 1);
        assert!(
            matches!(&received[0], AppCommand::InstallPlugin(url) if url == "https://github.com/owner/repo")
        );
    }

    #[test]
    fn test_handle_installing_plugin_empty_url_does_not_submit() {
        let mut state = setup_install_plugin_state("", 0);
        let (tx, rx) = channel();
        let result = handle_installing_plugin(&mut state.ui, key_event(KEY_ENTER), &tx);
        assert!(result);

        let received: Vec<AppCommand> = rx.try_iter().collect();
        assert!(received.is_empty());
    }

    #[test]
    fn test_handle_installing_plugin_types_characters() {
        let mut state = setup_install_plugin_state("https://", 8);
        let (tx, _rx) = channel();
        handle_installing_plugin(&mut state.ui, key_event(KeyCode::Char('g')), &tx);
        handle_installing_plugin(&mut state.ui, key_event(KeyCode::Char('h')), &tx);
        if let Modal::InstallPlugin { url, cursor, .. } = &state.ui.modal {
            assert_eq!(url, "https://gh");
            assert_eq!(*cursor, 10);
        } else {
            panic!("Expected InstallPlugin modal");
        }
    }

    #[test]
    fn test_handle_installing_plugin_backspace() {
        let mut state = setup_install_plugin_state("https://g", 9);
        let (tx, _rx) = channel();
        handle_installing_plugin(&mut state.ui, key_event(KEY_BACKSPACE), &tx);
        if let Modal::InstallPlugin { url, cursor, .. } = &state.ui.modal {
            assert_eq!(url, "https://");
            assert_eq!(*cursor, 8);
        } else {
            panic!("Expected InstallPlugin modal");
        }
    }

    #[test]
    fn test_handle_installing_plugin_left_right_navigation() {
        let mut state = setup_install_plugin_state("abc", 1);
        let (tx, _rx) = channel();
        handle_installing_plugin(&mut state.ui, key_event(KeyCode::Left), &tx);
        if let Modal::InstallPlugin { cursor, .. } = &state.ui.modal {
            assert_eq!(*cursor, 0);
        } else {
            panic!("Expected InstallPlugin modal");
        }

        handle_installing_plugin(&mut state.ui, key_event(KeyCode::Right), &tx);
        if let Modal::InstallPlugin { cursor, .. } = &state.ui.modal {
            assert_eq!(*cursor, 1);
        } else {
            panic!("Expected InstallPlugin modal");
        }
    }

    #[test]
    fn test_handle_installing_plugin_home_end() {
        let mut state = setup_install_plugin_state("abcdef", 3);
        let (tx, _rx) = channel();
        handle_installing_plugin(&mut state.ui, key_event(KeyCode::Home), &tx);
        if let Modal::InstallPlugin { cursor, .. } = &state.ui.modal {
            assert_eq!(*cursor, 0);
        } else {
            panic!("Expected InstallPlugin modal");
        }

        handle_installing_plugin(&mut state.ui, key_event(KeyCode::End), &tx);
        if let Modal::InstallPlugin { cursor, .. } = &state.ui.modal {
            assert_eq!(*cursor, 6);
        } else {
            panic!("Expected InstallPlugin modal");
        }
    }

    #[test]
    fn test_handle_installing_plugin_delete() {
        let mut state = setup_install_plugin_state("abcd", 2);
        let (tx, _rx) = channel();
        handle_installing_plugin(&mut state.ui, key_event(KeyCode::Delete), &tx);
        if let Modal::InstallPlugin { url, cursor, .. } = &state.ui.modal {
            assert_eq!(url, "abd");
            assert_eq!(*cursor, 2);
        } else {
            panic!("Expected InstallPlugin modal");
        }
    }
}
