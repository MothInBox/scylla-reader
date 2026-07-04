//! Top-level app state — owns reader, library, settings, and modal substates.

pub mod modal;
pub use modal::Modal;
pub mod page;
pub use page::Page;
pub mod reader;
pub use reader::ReaderState;

use crate::db::Db;
use crate::library::Library;
use crate::settings::Settings;

pub struct AppState {
    pub library: Library,
    pub current_page: Page,
    pub modal: Modal,
    pub settings: Settings,
    pub reader: ReaderState,
    pub db: Db,
    pub show_hints: bool,
}
impl AppState {
    pub fn new() -> Self {
        Self {
            library: Library::new(),
            current_page: Page::Library,
            modal: Modal::None,
            settings: Settings::new(),
            reader: ReaderState::new(),
            db: Db::open().unwrap_or_else(|e| {
                crate::settings::log(
                    crate::settings::LogLevel::Error,
                    "DB",
                    &format!("DB open failed: {}", e),
                );
                panic!("Could not open database");
            }),
            show_hints: true,
        }
    }

    pub fn close_modal(&mut self) {
        self.modal = Modal::None;
    }

    pub fn valid_add_book_inputs(&self) -> Vec<String> {
        if let Modal::AddBook { inputs, .. } = &self.modal {
            inputs
                .iter()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        } else {
            vec![]
        }
    }
    #[cfg(test)]
    pub fn from_parts(db: crate::db::Db, library: crate::library::Library) -> Self {
        Self {
            library,
            current_page: Page::Library,
            modal: Modal::None,
            settings: Settings::new(),
            reader: ReaderState::new(),
            db,
            show_hints: true,
        }
    }

    pub fn open_reader_chapter(
        &mut self,
        chapter_title: String,
        content: String,
        chapter_idx: usize,
    ) {
        let book_title = self
            .library
            .selected_book()
            .map(|b| b.title.clone())
            .unwrap_or_default();
        let book_url = self
            .library
            .selected_book()
            .map(|b| b.url.clone())
            .unwrap_or_default();
        self.reader
            .load(book_title, book_url, chapter_title, content, chapter_idx);
        self.current_page = Page::Reader;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;

    fn test_state() -> AppState {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let db = Db::open_conn(conn).unwrap();
        AppState::from_parts(db, Library::new())
    }

    #[test]
    fn test_close_modal_sets_none() {
        let mut state = test_state();
        state.modal = Modal::AddBook {
            inputs: vec!["url".into()],
            cursor: 0,
            scroll_offset: 0,
        };
        state.close_modal();
        assert_eq!(state.modal, Modal::None);
    }

    #[test]
    fn test_valid_add_book_inputs_returns_filtered() {
        let mut state = test_state();
        state.modal = Modal::AddBook {
            inputs: vec!["  url1  ".into(), "".into(), "url2".into()],
            cursor: 0,
            scroll_offset: 0,
        };
        let urls = state.valid_add_book_inputs();
        assert_eq!(urls, vec!["url1", "url2"]);
    }

    #[test]
    fn test_valid_add_book_inputs_not_add_book_modal() {
        let state = test_state();
        assert!(state.valid_add_book_inputs().is_empty());
    }

    #[test]
    fn test_open_reader_chapter_with_selected_book() {
        let mut state = test_state();
        state.library.add_book("Test Book".into(), "url".into(), 10);
        state.open_reader_chapter("Ch1".into(), "content".into(), 0);
        assert_eq!(state.current_page, Page::Reader);
        assert_eq!(state.reader.book_title, "Test Book");
        assert_eq!(state.reader.chapter_title, "Ch1");
    }

    #[test]
    fn test_open_reader_chapter_without_selected_book() {
        let mut state = test_state();
        state.open_reader_chapter("Ch1".into(), "content".into(), 0);
        assert_eq!(state.current_page, Page::Reader);
        assert!(state.reader.book_title.is_empty());
    }

    #[test]
    fn test_show_hints_defaults_to_true() {
        let state = test_state();
        assert!(state.show_hints);
    }

    #[test]
    fn test_toggle_show_hints() {
        let mut state = test_state();
        assert!(state.show_hints);
        state.show_hints = false;
        assert!(!state.show_hints);
    }

    #[test]
    fn test_command_palette_modal_variant() {
        let modal = Modal::CommandPalette {
            query: String::new(),
            filtered: Vec::new(),
            selected: 0,
        };
        match modal {
            Modal::CommandPalette {
                query,
                filtered,
                selected,
            } => {
                assert!(query.is_empty());
                assert!(filtered.is_empty());
                assert_eq!(selected, 0);
            }
            _ => panic!("Expected CommandPalette variant"),
        }
    }

    #[test]
    fn test_page_transitions() {
        let mut state = test_state();
        assert_eq!(state.current_page, Page::Library);
        state.current_page = Page::Settings;
        assert_eq!(state.current_page, Page::Settings);
        state.current_page = Page::Library;
        assert_eq!(state.current_page, Page::Library);
    }
}
