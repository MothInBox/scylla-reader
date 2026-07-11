//! Top-level app state — owns reader, library, settings, and modal substates.

pub mod lib;
pub use lib::LibraryState;
pub mod modal;
pub use modal::Modal;
pub mod page;
pub use page::Page;
pub mod palette_action;
pub mod reader;
pub use reader::ReaderState;
pub mod ui;
pub use ui::UiState;
pub mod jobs;
pub use jobs::JobsState;
pub mod settings_ui;

use crate::db::Db;

pub struct AppState {
    pub(crate) ui: UiState,
    pub(crate) lib: LibraryState,
    pub(crate) reader: ReaderState,
    pub(crate) jobs: JobsState,
    pub(crate) db: Db,
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

impl AppState {
    pub fn new() -> Self {
        let mut state = Self {
            ui: UiState::new(),
            lib: LibraryState::new(),
            reader: ReaderState::new(),
            jobs: JobsState::new(),
            db: Db::open().unwrap_or_else(|e| {
                crate::settings::log(
                    crate::settings::LogLevel::Error,
                    "DB",
                    &format!("DB open failed: {}", e),
                );
                panic!("Could not open database");
            }),
        };
        state.jobs.max_workers = state.lib.settings.max_workers;
        state
    }

    pub fn close_modal(&mut self) {
        self.ui.modal = Modal::None;
    }

    #[cfg(test)]
    pub fn from_parts(db: crate::db::Db, library: crate::library::Library) -> Self {
        Self {
            ui: UiState::new(),
            lib: LibraryState {
                library,
                settings: crate::settings::Settings::new(),
                settings_ui: crate::state::settings_ui::SettingsUiState::new(),
            },
            reader: ReaderState::new(),
            jobs: JobsState::new(),
            db,
        }
    }

    pub fn set_page(&mut self, page: Page) {
        self.ui.page = page;
    }

    pub fn set_modal(&mut self, modal: Modal) {
        self.ui.modal = modal;
    }

    pub fn page(&self) -> &Page {
        &self.ui.page
    }

    pub fn modal(&self) -> &Modal {
        &self.ui.modal
    }

    pub fn open_reader_chapter(
        &mut self,
        chapter_title: String,
        content: String,
        chapter_idx: usize,
        session_id: i64,
        session_name: String,
    ) {
        let book_title = self
            .lib
            .library
            .selected_book()
            .map(|b| b.title.clone())
            .unwrap_or_default();
        let book_url = self
            .lib
            .library
            .selected_book()
            .map(|b| b.url.clone())
            .unwrap_or_default();
        self.reader
            .load(book_title, book_url, chapter_title, content, chapter_idx);
        self.reader.session_id = session_id;
        self.reader.session_name = session_name;
        self.ui.page = Page::Reader;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;
    use crate::library::Library;

    fn test_state() -> AppState {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let db = Db::open_conn(conn).unwrap();
        AppState::from_parts(db, Library::new())
    }

    #[test]
    fn test_close_modal_sets_none() {
        let mut state = test_state();
        state.ui.modal = Modal::AddBook {
            inputs: vec!["url".into()],
            cursor: 0,
            scroll_offset: 0,
        };
        state.close_modal();
        assert_eq!(state.ui.modal, Modal::None);
    }

    #[test]
    fn test_open_reader_chapter_with_selected_book() {
        let mut state = test_state();
        state.lib.library.add_book("Test Book".into(), "url".into());
        state.open_reader_chapter("Ch1".into(), "content".into(), 0, 1, "default".into());
        assert_eq!(state.ui.page, Page::Reader);
        assert_eq!(state.reader.book_title, "Test Book");
        assert_eq!(state.reader.chapter_title, "Ch1");
        assert_eq!(state.reader.session_id, 1);
        assert_eq!(state.reader.session_name, "default");
    }

    #[test]
    fn test_open_reader_chapter_without_selected_book() {
        let mut state = test_state();
        state.open_reader_chapter("Ch1".into(), "content".into(), 0, 1, "default".into());
        assert_eq!(state.ui.page, Page::Reader);
        assert!(state.reader.book_title.is_empty());
    }

    #[test]
    fn test_show_hints_defaults_to_true() {
        let state = test_state();
        assert!(state.ui.show_hints);
    }

    #[test]
    fn test_toggle_show_hints() {
        let mut state = test_state();
        assert!(state.ui.show_hints);
        state.ui.show_hints = false;
        assert!(!state.ui.show_hints);
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
        assert_eq!(state.ui.page, Page::Library);
        state.ui.page = Page::Settings;
        assert_eq!(state.ui.page, Page::Settings);
        state.ui.page = Page::Library;
        assert_eq!(state.ui.page, Page::Library);
    }
}
