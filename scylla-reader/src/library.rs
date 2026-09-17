//! Book library — in-memory collection with filtering, selection, and
//! cover-image caching.

use crate::models::{Book, BookStatus};
use crate::storage::client::EmbeddingStatus;
use ratatui_image::protocol::StatefulProtocol;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct BookFilter {
    pub name: String,               // "" = no name constraint
    pub tags: Vec<String>,          // any-match (OR); empty = no tag constraint
    pub status: Option<BookStatus>, // None = any status
    pub library: Option<String>,    // None = primary backend (display + scope)
}

impl BookFilter {
    pub fn is_empty(&self) -> bool {
        self.name.trim().is_empty()
            && self.tags.is_empty()
            && self.status.is_none()
            && self.library.is_none()
    }

    /// Position of `status` in the status choice list
    /// `[None, Reading, Paused, Dropped, Completed]`.
    pub fn status_cursor(&self) -> usize {
        match &self.status {
            None => 0,
            Some(BookStatus::Reading) => 1,
            Some(BookStatus::Paused) => 2,
            Some(BookStatus::Dropped) => 3,
            Some(BookStatus::Completed) => 4,
        }
    }

    /// Position of `library` in the library choice list
    /// `[None, ...backend_names]` (None → 0, name → index + 1).
    pub fn library_cursor(&self, backend_names: &[String]) -> usize {
        match &self.library {
            None => 0,
            Some(name) => backend_names
                .iter()
                .position(|n| n == name)
                .map_or(0, |i| i + 1),
        }
    }
}

impl std::fmt::Display for BookFilter {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        if self.is_empty() {
            return write!(f, "All");
        }
        let mut segments: Vec<String> = Vec::new();
        if !self.name.trim().is_empty() {
            segments.push(format!("\"{}\"", self.name));
        }
        if !self.tags.is_empty() {
            segments.push(
                self.tags
                    .iter()
                    .map(|t| format!("#{}", t))
                    .collect::<Vec<_>>()
                    .join(" "),
            );
        }
        if let Some(status) = &self.status {
            segments.push(status.to_string());
        }
        if let Some(library) = &self.library {
            segments.push(format!("@{}", library));
        }
        write!(f, "{}", segments.join(" · "))
    }
}

pub struct Library {
    pub books: Vec<Book>,
    pub selected_index: usize,
    pub filter: BookFilter,
    pub search_order: Option<Vec<usize>>,
    pub cover_cache: HashMap<String, StatefulProtocol>,
    pub embedding_status_cache: HashMap<String, EmbeddingStatus>,
    /// When each book's embedding status was last fetched (for periodic refresh).
    pub embedding_status_fetched_at: HashMap<String, std::time::Instant>,
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
            filter: BookFilter::default(),
            search_order: None,
            cover_cache: HashMap::new(),
            embedding_status_cache: HashMap::new(),
            embedding_status_fetched_at: HashMap::new(),
        }
    }

    pub fn visible_indices(&self) -> Vec<usize> {
        if let Some(order) = &self.search_order {
            order.clone()
        } else {
            self.search(&self.filter)
        }
    }

    /// Predicate filter over `self.books` in library order. The `library`
    /// facet is a data-source switch (books are reloaded from the backend),
    /// not a per-book predicate, so it does not filter the list here.
    pub fn search(&self, filter: &BookFilter) -> Vec<usize> {
        self.books
            .iter()
            .enumerate()
            .filter(|(_, book)| {
                let name_ok = filter.name.trim().is_empty()
                    || book
                        .title
                        .to_lowercase()
                        .contains(&filter.name.trim().to_lowercase());
                let tags_ok =
                    filter.tags.is_empty() || filter.tags.iter().any(|t| book.tags.contains(t));
                let status_ok = filter.status.as_ref().is_none_or(|s| &book.status == s);
                name_ok && tags_ok && status_ok
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

    pub fn add_book(&mut self, title: String, url: String) {
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
            // The search order holds indices into the old `books` layout;
            // drop it so `visible_indices()` recomputes from the filter.
            self.search_order = None;
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
        // Cycling the last visible book's status can remove it from the view;
        // clamp the selection so it never dangles out of bounds.
        let len = self.visible_indices().len();
        if self.selected_index >= len {
            self.selected_index = len.saturating_sub(1);
        }
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

/// Union of all book tags, deduped and sorted.
pub fn known_tags(books: &[Book]) -> Vec<String> {
    let mut tags: Vec<String> = Vec::new();
    for book in books {
        for tag in &book.tags {
            if !tags.contains(tag) {
                tags.push(tag.clone());
            }
        }
    }
    tags.sort();
    tags
}

/// Lowercase substring filter over tags; an empty query returns all tags.
pub fn filter_tags(tags: &[String], query: &str) -> Vec<String> {
    if query.trim().is_empty() {
        return tags.to_vec();
    }
    let q = query.to_lowercase();
    tags.iter()
        .filter(|t| t.to_lowercase().contains(&q))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lib_with_n(n: usize) -> Library {
        let mut lib = Library::new();
        for i in 0..n {
            lib.add_book(format!("Book {}", i), format!("url-{}", i));
        }
        lib
    }

    #[test]
    fn test_new_library_is_empty() {
        let lib = Library::new();
        assert!(lib.books.is_empty());
        assert_eq!(lib.selected_index, 0);
        assert_eq!(lib.filter, BookFilter::default());
        assert!(lib.search_order.is_none());
    }

    #[test]
    fn test_add_one_book() {
        let mut lib = Library::new();
        lib.add_book("Title".into(), "url".into());
        assert_eq!(lib.books.len(), 1);
        assert_eq!(lib.books[0].title, "Title");
        assert_eq!(lib.books[0].url, "url");
        assert!(lib.books[0].sessions.is_empty());
        assert_eq!(lib.books[0].status, BookStatus::Reading);
    }

    #[test]
    fn test_add_five_books() {
        let mut lib = Library::new();
        for title in ["Book A", "Book B", "Book C", "Book D", "Book E"] {
            lib.add_book(title.to_string(), String::new());
        }
        assert_eq!(lib.books.len(), 5);
        assert_eq!(lib.books[0].title, "Book A");
        assert!(lib.books[4].sessions.is_empty());
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
    fn test_remove_selected_clears_stale_search_order() {
        let mut lib = lib_with_n(3);
        lib.search_order = Some(vec![2, 0, 1]);
        lib.remove_selected();
        assert!(lib.search_order.is_none());
        // Second removal must not panic on the stale order.
        lib.remove_selected();
        assert!(lib.search_order.is_none());
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
        lib.filter.status = Some(BookStatus::Reading);
        assert_eq!(lib.visible_indices().len(), 2);
        assert_eq!(lib.visible_indices(), vec![0, 2]);
    }

    #[test]
    fn test_visible_indices_all_dropped() {
        let mut lib = lib_with_n(3);
        lib.books[0].status = BookStatus::Dropped;
        lib.books[1].status = BookStatus::Dropped;
        lib.books[2].status = BookStatus::Dropped;
        lib.filter.status = Some(BookStatus::Completed);
        assert!(lib.visible_indices().is_empty());
    }

    #[test]
    fn test_filter_tags_matches() {
        let mut lib = lib_with_n(3);
        lib.books[0].tags.push("fantasy".into());
        lib.books[2].tags.push("fantasy".into());
        lib.filter.tags = vec!["fantasy".into()];
        assert_eq!(lib.visible_indices(), vec![0, 2]);
    }

    #[test]
    fn test_filter_tags_no_match() {
        let mut lib = lib_with_n(3);
        lib.filter.tags = vec!["nonexistent".into()];
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
    fn test_cycle_selected_status_clamps_when_book_leaves_view() {
        let mut lib = lib_with_n(2);
        lib.books[0].status = BookStatus::Reading;
        lib.books[1].status = BookStatus::Reading;
        lib.filter.status = Some(BookStatus::Reading);
        lib.selected_index = 1;
        lib.cycle_selected_status();
        assert_eq!(lib.books[1].status, BookStatus::Paused);
        assert_eq!(lib.selected_index, 0);
    }

    #[test]
    fn test_navigation_respects_filter_bounds() {
        let mut lib = Library::new();
        lib.add_book("Book A".into(), "url-a".into());
        lib.add_book("Book B".into(), "url-b".into());
        lib.books[1].status = BookStatus::Dropped;

        lib.filter.status = Some(BookStatus::Reading);

        lib.selected_index = 0;
        lib.select_next();
        assert_eq!(
            lib.selected_index, 0,
            "should not navigate past visible items"
        );
    }

    #[test]
    fn test_search_by_name() {
        let mut lib = lib_with_n(3);
        lib.books[0].title = "The Glorious Dawn".into();
        lib.books[1].title = "Gloom".into();
        lib.books[2].title = "Other".into();
        let filter = BookFilter {
            name: "glo".into(),
            ..Default::default()
        };
        assert_eq!(lib.search(&filter), vec![0, 1]);
    }

    #[test]
    fn test_search_name_case_insensitive_and_trimmed() {
        let mut lib = lib_with_n(2);
        lib.books[0].title = "GLORIOUS".into();
        let filter = BookFilter {
            name: "  glorious  ".into(),
            ..Default::default()
        };
        assert_eq!(lib.search(&filter), vec![0]);
    }

    #[test]
    fn test_search_by_status() {
        let mut lib = lib_with_n(3);
        lib.books[1].status = BookStatus::Dropped;
        let filter = BookFilter {
            status: Some(BookStatus::Dropped),
            ..Default::default()
        };
        assert_eq!(lib.search(&filter), vec![1]);
    }

    #[test]
    fn test_search_by_tag_any_match() {
        let mut lib = lib_with_n(3);
        lib.books[0].tags = vec!["fantasy".into()];
        lib.books[1].tags = vec!["litrpg".into()];
        lib.books[2].tags = vec!["scifi".into()];
        let filter = BookFilter {
            tags: vec!["fantasy".into(), "litrpg".into()],
            ..Default::default()
        };
        assert_eq!(lib.search(&filter), vec![0, 1]);
    }

    #[test]
    fn test_search_combined_name_tag_status() {
        let mut lib = lib_with_n(4);
        lib.books[0].title = "Dragon Heart".into();
        lib.books[0].tags = vec!["fantasy".into()];
        lib.books[0].status = BookStatus::Reading;
        lib.books[1].title = "Dragon Heart 2".into();
        lib.books[1].tags = vec!["fantasy".into()];
        lib.books[1].status = BookStatus::Dropped;
        lib.books[2].title = "Dragon".into();
        lib.books[2].tags = vec!["scifi".into()];
        lib.books[2].status = BookStatus::Reading;
        lib.books[3].title = "Heart".into();
        lib.books[3].tags = vec!["fantasy".into()];
        lib.books[3].status = BookStatus::Reading;
        let filter = BookFilter {
            name: "dragon".into(),
            tags: vec!["fantasy".into()],
            status: Some(BookStatus::Reading),
            library: None,
        };
        assert_eq!(lib.search(&filter), vec![0]);
    }

    #[test]
    fn test_search_empty_filter_returns_all() {
        let lib = lib_with_n(3);
        assert_eq!(lib.search(&BookFilter::default()), vec![0, 1, 2]);
    }

    #[test]
    fn test_search_library_facet_does_not_filter() {
        let lib = lib_with_n(3);
        let filter = BookFilter {
            library: Some("remote1".into()),
            ..Default::default()
        };
        assert_eq!(lib.search(&filter), vec![0, 1, 2]);
    }

    #[test]
    fn test_filter_is_empty() {
        assert!(BookFilter::default().is_empty());
        assert!(
            !BookFilter {
                name: "x".into(),
                ..Default::default()
            }
            .is_empty()
        );
        assert!(
            !BookFilter {
                tags: vec!["x".into()],
                ..Default::default()
            }
            .is_empty()
        );
        assert!(
            !BookFilter {
                status: Some(BookStatus::Reading),
                ..Default::default()
            }
            .is_empty()
        );
        assert!(
            !BookFilter {
                library: Some("r".into()),
                ..Default::default()
            }
            .is_empty()
        );
    }

    #[test]
    fn test_filter_is_empty_trims_whitespace_name() {
        assert!(
            BookFilter {
                name: "   ".into(),
                ..Default::default()
            }
            .is_empty()
        );
    }

    #[test]
    fn test_status_cursor_positions() {
        let f = |s: Option<BookStatus>| BookFilter {
            status: s,
            ..Default::default()
        };
        assert_eq!(f(None).status_cursor(), 0);
        assert_eq!(f(Some(BookStatus::Reading)).status_cursor(), 1);
        assert_eq!(f(Some(BookStatus::Paused)).status_cursor(), 2);
        assert_eq!(f(Some(BookStatus::Dropped)).status_cursor(), 3);
        assert_eq!(f(Some(BookStatus::Completed)).status_cursor(), 4);
    }

    #[test]
    fn test_library_cursor_positions() {
        let names = vec!["a".to_string(), "b".to_string()];
        let f = |l: Option<String>| BookFilter {
            library: l,
            ..Default::default()
        };
        assert_eq!(f(None).library_cursor(&names), 0);
        assert_eq!(f(Some("a".into())).library_cursor(&names), 1);
        assert_eq!(f(Some("b".into())).library_cursor(&names), 2);
        assert_eq!(f(Some("missing".into())).library_cursor(&names), 0);
    }

    #[test]
    fn test_filter_display_all() {
        assert_eq!(format!("{}", BookFilter::default()), "All");
    }

    #[test]
    fn test_filter_display_name() {
        let filter = BookFilter {
            name: "glo".into(),
            ..Default::default()
        };
        assert_eq!(format!("{}", filter), "\"glo\"");
    }

    #[test]
    fn test_filter_display_tags() {
        let filter = BookFilter {
            tags: vec!["fantasy".into(), "litrpg".into()],
            ..Default::default()
        };
        assert_eq!(format!("{}", filter), "#fantasy #litrpg");
    }

    #[test]
    fn test_filter_display_status() {
        let filter = BookFilter {
            status: Some(BookStatus::Reading),
            ..Default::default()
        };
        assert_eq!(format!("{}", filter), "Reading");
    }

    #[test]
    fn test_filter_display_library() {
        let filter = BookFilter {
            library: Some("remote1".into()),
            ..Default::default()
        };
        assert_eq!(format!("{}", filter), "@remote1");
    }

    #[test]
    fn test_filter_display_combined() {
        let filter = BookFilter {
            name: "glo".into(),
            tags: vec!["fantasy".into()],
            status: Some(BookStatus::Reading),
            library: Some("remote1".into()),
        };
        assert_eq!(
            format!("{}", filter),
            "\"glo\" · #fantasy · Reading · @remote1"
        );
    }

    #[test]
    fn test_search_order_overrides_predicate_filter() {
        let mut lib = lib_with_n(3);
        lib.books[1].status = BookStatus::Dropped;
        lib.filter.status = Some(BookStatus::Reading);
        lib.search_order = Some(vec![1, 0]);
        assert_eq!(lib.visible_indices(), vec![1, 0]);
    }

    #[test]
    fn test_known_tags_union_dedupe_sort() {
        let mut lib = lib_with_n(2);
        lib.books[0].tags = vec!["litrpg".into(), "fantasy".into()];
        lib.books[1].tags = vec!["fantasy".into(), "scifi".into()];
        assert_eq!(known_tags(&lib.books), vec!["fantasy", "litrpg", "scifi"]);
    }

    #[test]
    fn test_known_tags_empty() {
        assert!(known_tags(&[]).is_empty());
    }

    #[test]
    fn test_filter_tags_empty_query_returns_all() {
        let tags = vec!["fantasy".into(), "litrpg".into()];
        assert_eq!(filter_tags(&tags, ""), tags);
        assert_eq!(filter_tags(&tags, "   "), tags);
    }

    #[test]
    fn test_filter_tags_substring_case_insensitive() {
        let tags = vec!["Fantasy".into(), "Litrpg".into(), "Scifi".into()];
        assert_eq!(filter_tags(&tags, "fan"), vec!["Fantasy"]);
        assert_eq!(filter_tags(&tags, "FAN"), vec!["Fantasy"]);
        assert_eq!(filter_tags(&tags, "rp"), vec!["Litrpg"]);
        assert_eq!(filter_tags(&tags, "zzz"), Vec::<String>::new());
    }
}
