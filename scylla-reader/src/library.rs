//! Book library — in-memory collection with filtering, selection, and
//! cover-image caching.

use crate::models::{Book, BookStatus};
use ratatui_image::protocol::StatefulProtocol;

#[derive(Debug, PartialEq, Clone)]
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
    pub cached_protocol: Option<StatefulProtocol>,
}

impl Default for Library {
    fn default() -> Self {
        Self::new()
    }
}

impl Library {
    pub fn new() -> Self {
        Self {
            books: Vec::new(),
            selected_index: 0,
            filter: LibraryFilter::All,
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

    pub fn add_book(&mut self, title: String, url: String, _total_pages: u32) {
        self.books.push(Book {
            title,
            url,
            status: BookStatus::Reading,
            sessions: Vec::new(),
            active_session_id: None,
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
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lib_with_n(n: usize) -> Library {
        let mut lib = Library::new();
        for i in 0..n {
            lib.add_book(format!("Book {}", i), format!("url-{}", i), 100);
        }
        lib
    }

    #[test]
    fn test_new_library_is_empty() {
        let lib = Library::new();
        assert!(lib.books.is_empty());
        assert_eq!(lib.selected_index, 0);
        assert_eq!(lib.filter, LibraryFilter::All);
    }

    #[test]
    fn test_add_one_book() {
        let mut lib = Library::new();
        lib.add_book("Title".into(), "url".into(), 42);
        assert_eq!(lib.books.len(), 1);
        assert_eq!(lib.books[0].title, "Title");
        assert_eq!(lib.books[0].url, "url");
        assert_eq!(lib.books[0].progress.total, 42);
        assert_eq!(lib.books[0].status, BookStatus::Reading);
    }

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
    fn test_selected_book_returns_none_when_empty() {
        let lib = Library::new();
        assert!(lib.selected_book().is_none());
    }

    #[test]
    fn test_selected_book_returns_some() {
        let lib = lib_with_n(3);
        assert_eq!(lib.selected_book().unwrap().title, "Book 0");
    }

    #[test]
    fn test_selected_book_mut_returns_none_when_empty() {
        let mut lib = Library::new();
        assert!(lib.selected_book_mut().is_none());
    }

    #[test]
    fn test_selected_book_mut_allows_mutation() {
        let mut lib = lib_with_n(3);
        lib.selected_book_mut().unwrap().title = "Changed".to_string();
        assert_eq!(lib.books[0].title, "Changed");
    }

    #[test]
    fn test_select_next_increments() {
        let mut lib = lib_with_n(5);
        lib.select_next();
        assert_eq!(lib.selected_index, 1);
    }

    #[test]
    fn test_select_next_clamps() {
        let mut lib = lib_with_n(3);
        lib.selected_index = 2;
        lib.select_next();
        assert_eq!(lib.selected_index, 2);
    }

    #[test]
    fn test_select_prev_decrements() {
        let mut lib = lib_with_n(5);
        lib.selected_index = 3;
        lib.select_prev();
        assert_eq!(lib.selected_index, 2);
    }

    #[test]
    fn test_select_prev_clamps() {
        let mut lib = lib_with_n(5);
        lib.select_prev();
        assert_eq!(lib.selected_index, 0);
    }

    #[test]
    fn test_remove_books() {
        let mut lib = lib_with_n(5);
        lib.selected_index = 2;
        lib.remove_selected();
        assert_eq!(lib.books.len(), 4);
        assert_eq!(lib.books[2].title, "Book 3");
        lib.selected_index = 3;
        lib.remove_selected();
        assert_eq!(lib.books.len(), 3);
    }

    #[test]
    fn test_remove_selected_from_empty_does_nothing() {
        let mut lib = Library::new();
        lib.selected_index = 0;
        lib.remove_selected();
        assert!(lib.books.is_empty());
    }

    #[test]
    fn test_remove_last_book_adjusts_index() {
        let mut lib = lib_with_n(3);
        lib.selected_index = 2;
        lib.remove_selected();
        assert_eq!(lib.selected_index, 1);
    }

    #[test]
    fn test_remove_first_book() {
        let mut lib = lib_with_n(3);
        lib.selected_index = 0;
        lib.remove_selected();
        assert_eq!(lib.books.len(), 2);
        assert_eq!(lib.books[0].title, "Book 1");
    }

    #[test]
    fn test_visible_indices_all() {
        let lib = lib_with_n(3);
        assert_eq!(lib.visible_indices(), vec![0, 1, 2]);
    }

    #[test]
    fn test_filter_by_status() {
        let mut lib = lib_with_n(3);
        lib.books[1].status = BookStatus::Dropped;
        lib.filter = LibraryFilter::ByStatus(BookStatus::Reading);
        assert_eq!(lib.visible_indices().len(), 2);
        assert_eq!(lib.visible_indices(), vec![0, 2]);
    }

    #[test]
    fn test_visible_indices_all_dropped() {
        let mut lib = lib_with_n(3);
        lib.books[0].status = BookStatus::Dropped;
        lib.books[1].status = BookStatus::Dropped;
        lib.books[2].status = BookStatus::Dropped;
        lib.filter = LibraryFilter::ByStatus(BookStatus::Completed);
        assert!(lib.visible_indices().is_empty());
    }

    #[test]
    fn test_filter_by_tag_matches() {
        let mut lib = lib_with_n(3);
        lib.books[0].tags.push("fantasy".into());
        lib.books[2].tags.push("fantasy".into());
        lib.filter_by_tag("fantasy".into());
        assert_eq!(lib.visible_indices(), vec![0, 2]);
    }

    #[test]
    fn test_filter_by_tag_no_match() {
        let mut lib = lib_with_n(3);
        lib.filter_by_tag("nonexistent".into());
        assert!(lib.visible_indices().is_empty());
    }

    #[test]
    fn test_cycle_status() {
        let mut lib = lib_with_n(1);
        lib.cycle_selected_status();
        assert_eq!(lib.books[0].status, BookStatus::Paused);
        lib.cycle_selected_status();
        assert_eq!(lib.books[0].status, BookStatus::Dropped);
    }

    #[test]
    fn test_cycle_selected_status_no_book() {
        let mut lib = Library::new();
        lib.cycle_selected_status();
    }

    #[test]
    fn test_cycle_filter_full_cycle() {
        let mut lib = Library::new();
        assert_eq!(lib.filter, LibraryFilter::All);
        lib.cycle_filter();
        assert_eq!(lib.filter, LibraryFilter::ByStatus(BookStatus::Reading));
        lib.cycle_filter();
        assert_eq!(lib.filter, LibraryFilter::ByStatus(BookStatus::Paused));
        lib.cycle_filter();
        assert_eq!(lib.filter, LibraryFilter::ByStatus(BookStatus::Dropped));
        lib.cycle_filter();
        assert_eq!(lib.filter, LibraryFilter::ByStatus(BookStatus::Completed));
        lib.cycle_filter();
        assert_eq!(lib.filter, LibraryFilter::All); // wraps
    }

    #[test]
    fn test_cycle_filter_from_tag_goes_to_all() {
        let mut lib = Library::new();
        lib.filter_by_tag("test".into());
        assert_eq!(lib.filter, LibraryFilter::ByTag("test".into()));
        lib.cycle_filter();
        assert_eq!(lib.filter, LibraryFilter::All);
    }

    #[test]
    fn test_cycle_filter_resets_index() {
        let mut lib = lib_with_n(5);
        lib.selected_index = 3;
        lib.cycle_filter();
        assert_eq!(lib.selected_index, 0);
    }

    #[test]
    fn test_filter_by_tag_resets_index() {
        let mut lib = lib_with_n(5);
        lib.selected_index = 3;
        lib.filter_by_tag("x".into());
        assert_eq!(lib.selected_index, 0);
    }

    #[test]
    fn test_navigation_respects_filter_bounds() {
        let mut lib = Library::new();
        lib.add_book("Book A".into(), "url-a".into(), 10);
        lib.add_book("Book B".into(), "url-b".into(), 10);
        lib.books[1].status = BookStatus::Dropped;

        lib.filter = LibraryFilter::ByStatus(BookStatus::Reading);

        lib.selected_index = 0;
        lib.select_next();
        assert_eq!(
            lib.selected_index, 0,
            "should not navigate past visible items"
        );
    }
}
