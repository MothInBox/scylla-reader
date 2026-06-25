pub mod modal;
pub use modal::Modal;
pub mod page;
pub use page::Page;
pub mod reader;
pub use reader::ReaderState;

use crate::db::Db;
use crate::library::Library;
use crate::models::Chapter;
use crate::settings::Settings;

pub struct AppState {
    pub library: Library,
    pub current_page: Page,
    pub modal: Modal,
    pub settings: Settings,
    pub reader: ReaderState,
    pub db: Db,
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
                crate::settings::log_debug(&format!("DB open failed: {}", e));
                panic!("Could not open database");
            }),
        }
    }

    pub fn close_modal(&mut self) {
        self.modal = Modal::None;
    }

    pub fn open_add_book_modal(&mut self) {
        self.modal = Modal::AddBook {
            inputs: vec![String::new()], // Assumes Modal uses Vec<String> now
            cursor: 0,
            scroll_offset: 0,
        };
    }

    pub fn open_jump_chapter_modal(&mut self, chapters: Vec<Chapter>) {
        self.modal = Modal::JumpChapter {
            chapters,
            cursor: 0,
            scroll_offset: 0,
        };
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
