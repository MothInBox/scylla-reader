use crate::state::{AppState, Modal};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub fn handle_key(state: &mut AppState, key: KeyEvent) -> bool {
    if let Modal::AddLibrary {
        ref mut name,
        ref mut url,
        cursor: _,
        ref mut focused_field,
    } = state.ui.modal
    {
        match key.code {
            KeyCode::Tab => {
                *focused_field = (*focused_field + 1) % 2;
                true
            }
            KeyCode::Enter => {
                if !name.is_empty() && !url.is_empty() {
                    let config = scylla_core::types::LibraryConfig {
                        name: name.clone(),
                        url: url.clone(),
                        default: false,
                    };
                    let mut libraries = crate::storage::config::load_libraries();
                    libraries.push(config.clone());
                    let _ = crate::storage::config::save_libraries(&libraries);

                    let backend = Box::new(crate::storage::remote::RemoteApi::new(
                        config.name,
                        config.url,
                    ))
                        as Box<dyn scylla_core::types::StorageBackend>;
                    state.lib.manager.backends.push(backend);
                }
                state.close_modal();
                true
            }
            KeyCode::Esc => {
                state.close_modal();
                true
            }
            KeyCode::Backspace => {
                if *focused_field == 0 {
                    name.pop();
                } else {
                    url.pop();
                }
                true
            }
            KeyCode::Char(c)
                if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
            {
                if *focused_field == 0 {
                    name.push(c);
                } else {
                    url.push(c);
                }
                true
            }
            _ => true,
        }
    } else {
        false
    }
}
