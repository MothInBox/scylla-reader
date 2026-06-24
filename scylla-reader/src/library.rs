use crate::models::{Book, BookStatus, Progress};
use image::DynamicImage;
use ratatui_image::protocol::StatefulProtocol;

#[derive(PartialEq, Clone)]
pub enum LibraryFilter {
    All,
    ByStatus(BookStatus),
    ByTag(String),
}

impl std::fmt::Display for LibraryFilter {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            LibraryFilter::All => write!(f, "All"),
            LibraryFilter::ByStatus(s) => write!(f, "{}", s),
            LibraryFilter::ByTag(t) => write!(f, "#{}", t),
        }
    }
}

pub struct Library {
    pub books: Vec<Book>,
    pub selected_index: usize,
    pub filter: LibraryFilter,
    pub cached_cover: Option<DynamicImage>,
    pub cached_cover_url: Option<String>,
    pub cached_protocol: Option<StatefulProtocol>,
}

impl Library {
    pub fn new() -> Self {
        Self {
            books: Vec::new(),
            selected_index: 0,
            filter: LibraryFilter::All,
            cached_cover: None,
            cached_cover_url: None,
            cached_protocol: None,
        }
    }

    pub fn visible_indices(&self) -> Vec<usize> {
        self.books
            .iter()
            .enumerate()
            .filter(|(_, b)| match &self.filter {
                LibraryFilter::All => true,
                LibraryFilter::ByStatus(s) => &b.status == s,
                LibraryFilter::ByTag(t) => b.tags.iter().any(|tag| tag == t),
            })
            .map(|(i, _)| i)
            .collect()
    }

    pub fn selected_book(&self) -> Option<&Book> {
        let indices = self.visible_indices();
        indices
            .get(self.selected_index)
            .and_then(|&i| self.books.get(i))
    }

    pub fn selected_book_mut(&mut self) -> Option<&mut Book> {
        let indices = self.visible_indices();
        let real_idx = *indices.get(self.selected_index)?;
        self.books.get_mut(real_idx)
    }

    pub fn add_book(&mut self, title: String, url: String, total_pages: u32) {
        self.books.push(Book {
            title,
            url,
            status: BookStatus::Reading,
            progress: Progress {
                current: 0,
                total: total_pages,
            },
            tags: Vec::new(),
            cover_url: None,
            description: None,
            chapters: Vec::new(),
        });
    }

    pub fn remove_selected(&mut self) {
        let indices = self.visible_indices();
        if let Some(&real_idx) = indices.get(self.selected_index) {
            self.books.remove(real_idx);
            let new_len = self.visible_indices().len();
            if self.selected_index > 0 && self.selected_index >= new_len {
                self.selected_index -= 1;
            }
        }
    }

    pub fn cycle_selected_status(&mut self) {
        if let Some(book) = self.selected_book_mut() {
            book.status = book.status.next();
        }
    }

    pub fn cycle_filter(&mut self) {
        self.filter = match &self.filter {
            LibraryFilter::All => LibraryFilter::ByStatus(BookStatus::Reading),
            LibraryFilter::ByStatus(BookStatus::Reading) => {
                LibraryFilter::ByStatus(BookStatus::Paused)
            }
            LibraryFilter::ByStatus(BookStatus::Paused) => {
                LibraryFilter::ByStatus(BookStatus::Dropped)
            }
            LibraryFilter::ByStatus(BookStatus::Dropped) => {
                LibraryFilter::ByStatus(BookStatus::Completed)
            }
            LibraryFilter::ByStatus(BookStatus::Completed) => LibraryFilter::All,
            LibraryFilter::ByTag(_) => LibraryFilter::All,
        };
        self.selected_index = 0;
    }

    pub fn filter_by_tag(&mut self, tag: String) {
        self.filter = LibraryFilter::ByTag(tag);
        self.selected_index = 0;
    }

    pub fn select_next(&mut self) {
        let len = self.visible_indices().len();
        if self.selected_index < len.saturating_sub(1) {
            self.selected_index += 1;
        }
    }

    pub fn select_prev(&mut self) {
        if self.selected_index > 0 {
            self.selected_index -= 1;
        }
    }

    pub fn set_chapter(
        &mut self,
        booktochange: Option<&mut Book>,
        new_chapter: &u32,
    ) -> Result<(), ()> {
        // Extract the mutable reference safely once
        if let Some(book) = booktochange {
            if book.progress.total >= *new_chapter {
                book.progress.current = *new_chapter;
                Ok(())
            } else {
                Err(())
            }
        } else {
            Err(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add_five_books() {
        let mut lib = Library::new();
        for title in ["Book A", "Book B", "Book C", "Book D", "Book E"] {
            lib.add_book(title.to_string(), String::new(), 100);
        }
        assert_eq!(lib.books.len(), 5);
        assert_eq!(lib.books[0].title, "Book A");
        assert_eq!(lib.books[4].progress.total, 100);
    }

    #[test]
    fn test_remove_books() {
        let mut lib = Library::new();
        for title in ["Book A", "Book B", "Book C", "Book D", "Book E"] {
            lib.add_book(title.to_string(), String::new(), 100);
        }
        lib.selected_index = 2;
        lib.remove_selected();
        assert_eq!(lib.books.len(), 4);
        assert_eq!(lib.books[2].title, "Book D");
        lib.selected_index = 3;
        lib.remove_selected();
        assert_eq!(lib.books.len(), 3);
    }

    #[test]
    fn test_filter_by_status() {
        let mut lib = Library::new();
        for title in ["Book A", "Book B", "Book C"] {
            lib.add_book(title.to_string(), String::new(), 100);
        }
        lib.books[1].status = BookStatus::Dropped;
        lib.filter = LibraryFilter::ByStatus(BookStatus::Reading);
        assert_eq!(lib.visible_indices().len(), 2);
    }

    #[test]
    fn test_cycle_status() {
        let mut lib = Library::new();
        lib.add_book("Book A".to_string(), String::new(), 100);
        lib.cycle_selected_status();
        assert_eq!(lib.books[0].status, BookStatus::Paused);
        lib.cycle_selected_status();
        assert_eq!(lib.books[0].status, BookStatus::Dropped);
    }
}
