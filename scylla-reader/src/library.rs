//! Book library — in-memory collection with filtering, selection, and
//! cover-image caching.

use crate::event_types::{AiBook, AiChapter, AiSearchResults, SearchOutcome};
use crate::models::{Book, BookStatus};
use crate::state::modal::SearchStatus;
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
    /// Explicit AI session — the source of truth for AI-ranked display. When
    /// `Some`, the library is ranked by the AI results; clearing is explicit
    /// (Esc / clear-AI), never a silent side effect of filter commit or delete.
    pub ai: Option<AiSession>,
    pub cover_cache: HashMap<String, StatefulProtocol>,
    pub embedding_status_cache: HashMap<String, EmbeddingStatus>,
    /// When each book's embedding status was last fetched (for periodic refresh).
    pub embedding_status_fetched_at: HashMap<String, std::time::Instant>,
}

/// A row in the AI results display. Book rows are shown in book mode; chapter
/// rows (under a book header) are shown in grouped-chapter mode / drill-down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiRow {
    Book {
        book_index: usize,
    },
    Chapter {
        book_index: usize,
        chapter_index: usize,
    },
}

/// Explicit AI search session. `book_mode` toggles between book rows (ranked
/// books) and grouped-chapter rows (book headers + their chapters); `cursor`
/// navigates the flattened [`AiRow`] list. `genre` is an optional local facet
/// (Phase 4) that narrows the ranked books to those carrying the genre — the
/// genres already travel in the response, so it needs no network round-trip.
#[derive(Debug, Clone, PartialEq)]
pub struct AiSession {
    pub query: String,
    pub status: SearchStatus,
    pub results: AiSearchResults,
    pub book_mode: bool,
    pub cursor: usize,
    pub genre: Option<String>,
}

impl AiSession {
    /// The flattened display rows for the current mode:
    /// - book mode: one book row per ranked book.
    /// - grouped-chapter mode: each book's header row followed by its chapters.
    ///
    /// Rows are limited to books matching the active `genre` facet (when set).
    pub fn rows(&self) -> Vec<AiRow> {
        let books = self.visible_book_indices();
        if self.book_mode {
            books
                .into_iter()
                .map(|bi| AiRow::Book { book_index: bi })
                .collect()
        } else {
            books
                .into_iter()
                .flat_map(|bi| {
                    let n = self.results.books[bi].chapters.len();
                    std::iter::once(AiRow::Book { book_index: bi }).chain((0..n).map(move |ci| {
                        AiRow::Chapter {
                            book_index: bi,
                            chapter_index: ci,
                        }
                    }))
                })
                .collect()
        }
    }

    /// Indices (into `self.results.books`) of the books passing the active
    /// `genre` facet. `None` genre → all books.
    pub fn visible_book_indices(&self) -> Vec<usize> {
        match &self.genre {
            None => (0..self.results.books.len()).collect(),
            Some(genre) => self
                .results
                .books
                .iter()
                .enumerate()
                .filter(|(_, b)| b.genres.iter().any(|g| g == genre))
                .map(|(i, _)| i)
                .collect(),
        }
    }

    /// Number of books visible under the active genre facet (for the chip's
    /// result count).
    pub fn visible_book_count(&self) -> usize {
        self.visible_book_indices().len()
    }

    /// The distinct genres across all ranked books, sorted (the cycle set for
    /// the genre chip). Empty when no book carries genres.
    pub fn available_genres(&self) -> Vec<String> {
        let mut genres: Vec<String> = Vec::new();
        for book in &self.results.books {
            for genre in &book.genres {
                if !genres.contains(genre) {
                    genres.push(genre.clone());
                }
            }
        }
        genres.sort();
        genres
    }

    /// Advance the genre facet to the next position in the cycle
    /// `none → g1 → g2 → … → gN → none` (wrapping in `dir`'s direction). No-op
    /// when no ranked book carries a genre. Resets the cursor so the selection
    /// lands on a visible row.
    pub fn cycle_genre(&mut self, dir: i32) {
        let genres = self.available_genres();
        if genres.is_empty() {
            self.genre = None;
            return;
        }
        // Position 0 is "no genre"; positions 1..=n map to `genres[i - 1]`.
        let current = match &self.genre {
            None => 0,
            Some(g) => genres
                .iter()
                .position(|x| x == g)
                .map(|i| i + 1)
                .unwrap_or(0),
        };
        let total = (genres.len() + 1) as i32;
        let next = (current as i32 + dir).rem_euclid(total) as usize;
        self.genre = if next == 0 {
            None
        } else {
            Some(genres[next - 1].clone())
        };
        self.cursor = 0;
    }
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
            ai: None,
            cover_cache: HashMap::new(),
            embedding_status_cache: HashMap::new(),
            embedding_status_fetched_at: HashMap::new(),
        }
    }

    pub fn visible_indices(&self) -> Vec<usize> {
        if let Some(order) = self.ai_search_order() {
            order
        } else {
            self.search(&self.filter)
        }
    }

    /// The AI-derived book order: ranked book indices into `self.books` (by
    /// `book_url`), in AI rank order. Returns `None` when no AI session is
    /// active. Books not present in the library are skipped. Honors the active
    /// `genre` facet (only books carrying that genre are ranked).
    pub fn ai_search_order(&self) -> Option<Vec<usize>> {
        let ai = self.ai.as_ref()?;
        Some(
            ai.visible_book_indices()
                .into_iter()
                .filter_map(|bi| {
                    self.books
                        .iter()
                        .position(|lb| lb.url == ai.results.books[bi].book_url)
                })
                .collect(),
        )
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
        if let Some(ai) = &self.ai {
            let book_url = self.ai_selected_book_url(ai)?;
            return self.books.iter().find(|b| b.url == book_url);
        }
        let indices = self.visible_indices();
        indices
            .get(self.selected_index)
            .and_then(|&i| self.books.get(i))
    }

    pub fn selected_book_mut(&mut self) -> Option<&mut Book> {
        if self.ai.is_some() {
            let book_url = {
                let ai = self.ai.as_ref()?;
                self.ai_selected_book_url(ai)?.to_string()
            };
            return self.books.iter_mut().find(|b| b.url == book_url);
        }
        let indices = self.visible_indices();
        let real_idx = *indices.get(self.selected_index)?;
        self.books.get_mut(real_idx)
    }

    /// The `book_url` of the currently selected AI row (book or chapter).
    fn ai_selected_book_url<'a>(&self, ai: &'a AiSession) -> Option<&'a str> {
        let rows = ai.rows();
        let row = rows.get(ai.cursor)?;
        let bi = match row {
            AiRow::Book { book_index } => *book_index,
            AiRow::Chapter { book_index, .. } => *book_index,
        };
        Some(ai.results.books[bi].book_url.as_str())
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
        // With an active AI session, remove the selected AI book from both the
        // library and the session (so the row doesn't dangle), then clamp the
        // cursor. The AI session itself is preserved (clearing is explicit, not
        // a side effect of delete).
        if self.ai.is_some() {
            let book_url = {
                let ai = self.ai.as_ref().unwrap();
                let rows = ai.rows();
                let Some(row) = rows.get(ai.cursor) else {
                    return;
                };
                let bi = match row {
                    AiRow::Book { book_index } => *book_index,
                    AiRow::Chapter { book_index, .. } => *book_index,
                };
                ai.results.books[bi].book_url.clone()
            };
            if let Some(pos) = self.books.iter().position(|b| b.url == book_url) {
                self.books.remove(pos);
            }
            if let Some(ai) = &mut self.ai {
                ai.results.books.retain(|b| b.book_url != book_url);
                let len = ai.rows().len();
                if ai.cursor >= len {
                    ai.cursor = len.saturating_sub(1);
                }
            }
            return;
        }
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

    // ── AI session lifecycle ────────────────────────────────────────────────

    /// Begin an AI search: activate a Loading session (results arrive later via
    /// the event channel). Closes any ranking seam so the library shows the
    /// active AI indicator immediately.
    pub fn set_ai_searching(&mut self, query: String) {
        self.ai = Some(AiSession {
            query,
            status: SearchStatus::Loading,
            results: AiSearchResults {
                embedded: 0,
                total: 0,
                books: Vec::new(),
            },
            book_mode: true,
            cursor: 0,
            genre: None,
        });
        self.selected_index = 0;
    }

    /// Apply a completed search outcome to the active AI session (normalizing
    /// whichever wire mode into ranked books). The session's display mode is
    /// preserved — a chapter-mode outcome must not reset a `g` toggle.
    pub fn apply_ai_results(&mut self, query: String, outcome: SearchOutcome) {
        let (status, results) = match outcome {
            SearchOutcome::BookMode {
                hits,
                embedded,
                total,
            } => {
                let status = if hits.is_empty() {
                    SearchStatus::Empty
                } else {
                    SearchStatus::Ready
                };
                (
                    status,
                    AiSearchResults {
                        embedded,
                        total,
                        books: hits,
                    },
                )
            }
            SearchOutcome::ChapterMode {
                hits,
                embedded,
                total,
            } => {
                let books = self.group_chapter_hits(&hits);
                let status = if books.is_empty() {
                    SearchStatus::Empty
                } else {
                    SearchStatus::Ready
                };
                (
                    status,
                    AiSearchResults {
                        embedded,
                        total,
                        books,
                    },
                )
            }
            SearchOutcome::NoEmbeddings { total } => (
                SearchStatus::NoEmbeddings { total },
                AiSearchResults {
                    embedded: 0,
                    total,
                    books: Vec::new(),
                },
            ),
        };
        let book_mode = self.ai.as_ref().map(|ai| ai.book_mode).unwrap_or(true);
        self.ai = Some(AiSession {
            query,
            status,
            results,
            book_mode,
            cursor: 0,
            genre: None,
        });
        self.selected_index = 0;
    }

    /// Mark the active AI session as failed (search error).
    pub fn set_ai_error(&mut self, query: String, error: String) {
        if self.ai.as_ref().is_none_or(|ai| ai.query != query) {
            return;
        }
        if let Some(ai) = &mut self.ai {
            ai.status = SearchStatus::Error(error);
        }
    }

    /// Explicitly clear the AI session and restore normal ranking.
    pub fn clear_ai(&mut self) {
        self.ai = None;
        self.selected_index = 0;
    }

    /// Toggle book-mode ↔ grouped-chapter mode.
    pub fn toggle_ai_mode(&mut self) {
        if let Some(ai) = &mut self.ai {
            ai.book_mode = !ai.book_mode;
            ai.cursor = 0;
        }
    }

    /// Cycle the genre facet through the ranked books' distinct genres
    /// (`dir` > 0 advances, < 0 retreats), wrapping back to none.
    pub fn cycle_ai_genre(&mut self, dir: i32) {
        if let Some(ai) = &mut self.ai {
            ai.cycle_genre(dir);
        }
    }

    /// Move the AI cursor through the flattened display rows.
    pub fn ai_nav(&mut self, dir: i32) {
        let len = self.ai.as_ref().map(|ai| ai.rows().len()).unwrap_or(0);
        if len == 0 {
            return;
        }
        if let Some(ai) = &mut self.ai {
            if dir > 0 && ai.cursor < len - 1 {
                ai.cursor += 1;
            } else if dir < 0 && ai.cursor > 0 {
                ai.cursor -= 1;
            }
        }
    }

    /// Group flat chapter-mode hits into ranked books (by mapping each chapter
    /// URL to its library book), sorted by score descending.
    fn group_chapter_hits(&self, hits: &[AiChapter]) -> Vec<AiBook> {
        // Map each chapter URL to its library book once (O(books × chapters))
        // instead of scanning the whole library per hit (O(hits × books ×
        // chapters)).
        let mut chapter_to_book: HashMap<&str, (&str, &str)> = HashMap::new();
        for book in &self.books {
            for chapter in &book.chapters {
                chapter_to_book
                    .entry(chapter.url.as_str())
                    .or_insert((book.url.as_str(), book.title.as_str()));
            }
        }
        let mut books: Vec<AiBook> = Vec::new();
        // book_url → index into `books`, so appending a chapter is O(1) rather
        // than a linear scan per hit.
        let mut book_index: HashMap<String, usize> = HashMap::new();
        for hit in hits {
            let (book_url, book_title) = chapter_to_book
                .get(hit.chapter_url.as_str())
                .map(|(u, t)| ((*u).to_string(), (*t).to_string()))
                .unwrap_or_else(|| (String::new(), "Unknown".to_string()));
            let chapter = AiChapter {
                chapter_url: hit.chapter_url.clone(),
                chapter_idx: hit.chapter_idx,
                chapter_title: hit.chapter_title.clone(),
                score: hit.score,
                snippet: hit.snippet.clone(),
            };
            if let Some(&idx) = book_index.get(&book_url) {
                let existing = &mut books[idx];
                existing.chapters.push(chapter);
                if hit.score > existing.score {
                    existing.score = hit.score;
                }
            } else {
                book_index.insert(book_url.clone(), books.len());
                books.push(AiBook {
                    book_url,
                    book_title,
                    score: hit.score,
                    genres: vec![],
                    chapters: vec![chapter],
                });
            }
        }
        for b in &mut books {
            b.chapters.sort_by(|a, c| {
                c.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
        }
        books.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        books
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
        assert!(lib.ai.is_none());
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

    // ── AI session ──────────────────────────────────────────────────────────

    /// A library with three books and a completed book-mode AI search that
    /// ranks them in reverse order (Book 2, Book 1, Book 0).
    fn ai_ranked_library() -> Library {
        let mut lib = lib_with_n(3);
        lib.apply_ai_results(
            "dragon heart".into(),
            SearchOutcome::BookMode {
                embedded: 3,
                total: 3,
                hits: vec![
                    AiBook {
                        book_url: "url-2".into(),
                        book_title: "Book 2".into(),
                        score: 90.0,
                        genres: vec![],
                        chapters: vec![AiChapter {
                            chapter_url: "url-2/ch0".into(),
                            chapter_idx: 0,
                            chapter_title: "Ch0".into(),
                            score: 90.0,
                            snippet: String::new(),
                        }],
                    },
                    AiBook {
                        book_url: "url-1".into(),
                        book_title: "Book 1".into(),
                        score: 70.0,
                        genres: vec![],
                        chapters: vec![],
                    },
                    AiBook {
                        book_url: "url-0".into(),
                        book_title: "Book 0".into(),
                        score: 50.0,
                        genres: vec![],
                        chapters: vec![],
                    },
                ],
            },
        );
        lib
    }

    #[test]
    fn test_new_library_has_no_ai() {
        let lib = Library::new();
        assert!(lib.ai.is_none());
    }

    #[test]
    fn test_set_ai_searching_activates_loading_session() {
        let mut lib = lib_with_n(3);
        lib.set_ai_searching("dragon".into());
        let ai = lib.ai.as_ref().unwrap();
        assert_eq!(ai.query, "dragon");
        assert_eq!(ai.status, SearchStatus::Loading);
        assert!(ai.book_mode);
        assert!(ai.results.books.is_empty());
        assert_eq!(ai.cursor, 0);
    }

    #[test]
    fn test_apply_book_mode_ranks_books_and_derives_order() {
        let lib = ai_ranked_library();
        let ai = lib.ai.as_ref().unwrap();
        assert_eq!(ai.status, SearchStatus::Ready);
        assert_eq!(ai.results.books.len(), 3);
        // Derived order follows the AI rank (Book 2, then 1, then 0).
        assert_eq!(lib.visible_indices(), vec![2, 1, 0]);
        assert_eq!(lib.ai_search_order(), Some(vec![2, 1, 0]));
    }

    #[test]
    fn test_book_mode_ai_overrides_filter() {
        let mut lib = ai_ranked_library();
        lib.filter.status = Some(BookStatus::Dropped);
        // AI-derived order wins over the predicate filter.
        assert_eq!(lib.visible_indices(), vec![2, 1, 0]);
    }

    #[test]
    fn test_book_mode_skips_books_not_in_library() {
        let mut lib = lib_with_n(1);
        lib.apply_ai_results(
            "q".into(),
            SearchOutcome::BookMode {
                embedded: 2,
                total: 2,
                hits: vec![
                    AiBook {
                        book_url: "url-0".into(),
                        book_title: "Book 0".into(),
                        score: 90.0,
                        genres: vec![],
                        chapters: vec![],
                    },
                    AiBook {
                        book_url: "not-in-library".into(),
                        book_title: "Ghost".into(),
                        score: 70.0,
                        genres: vec![],
                        chapters: vec![],
                    },
                ],
            },
        );
        // The ghost book is not in `self.books`, so the order only contains url-0.
        assert_eq!(lib.visible_indices(), vec![0]);
    }

    #[test]
    fn test_apply_chapter_mode_groups_hits_by_book() {
        let mut lib = lib_with_n(2);
        // Give Book 0 two chapters so chapter-mode hits map back to a book.
        lib.books[0].chapters = vec![
            crate::models::Chapter {
                title: "c0".into(),
                url: "url-0/ch0".into(),
                order: 0,
            },
            crate::models::Chapter {
                title: "c1".into(),
                url: "url-0/ch1".into(),
                order: 1,
            },
        ];
        lib.apply_ai_results(
            "q".into(),
            SearchOutcome::ChapterMode {
                embedded: 2,
                total: 2,
                hits: vec![
                    AiChapter {
                        chapter_url: "url-0/ch1".into(),
                        chapter_idx: 1,
                        chapter_title: "c1".into(),
                        score: 80.0,
                        snippet: String::new(),
                    },
                    AiChapter {
                        chapter_url: "url-0/ch0".into(),
                        chapter_idx: 0,
                        chapter_title: "c0".into(),
                        score: 90.0,
                        snippet: String::new(),
                    },
                ],
            },
        );
        let ai = lib.ai.as_ref().unwrap();
        assert_eq!(ai.results.books.len(), 1);
        assert_eq!(ai.results.books[0].book_title, "Book 0");
        // Book score = best chapter; chapters sorted by score desc.
        assert_eq!(ai.results.books[0].score, 90.0);
        assert_eq!(ai.results.books[0].chapters[0].chapter_idx, 0);
        assert_eq!(ai.results.books[0].chapters[1].chapter_idx, 1);
        assert_eq!(lib.visible_indices(), vec![0]);
    }

    #[test]
    fn test_apply_no_embeddings_sets_hint_status() {
        let mut lib = lib_with_n(2);
        lib.apply_ai_results("q".into(), SearchOutcome::NoEmbeddings { total: 12 });
        let ai = lib.ai.as_ref().unwrap();
        assert_eq!(ai.status, SearchStatus::NoEmbeddings { total: 12 });
        assert!(ai.results.books.is_empty());
    }

    #[test]
    fn test_apply_empty_books_sets_empty_status() {
        let mut lib = lib_with_n(2);
        lib.apply_ai_results(
            "q".into(),
            SearchOutcome::BookMode {
                embedded: 3,
                total: 3,
                hits: vec![],
            },
        );
        assert_eq!(lib.ai.as_ref().unwrap().status, SearchStatus::Empty);
    }

    #[test]
    fn test_clear_ai_restores_normal_ranking() {
        let mut lib = ai_ranked_library();
        assert_eq!(lib.visible_indices(), vec![2, 1, 0]);
        lib.clear_ai();
        assert!(lib.ai.is_none());
        assert_eq!(lib.visible_indices(), vec![0, 1, 2]);
    }

    #[test]
    fn test_toggle_ai_mode_switches_book_and_grouped() {
        let mut lib = ai_ranked_library();
        assert!(lib.ai.as_ref().unwrap().book_mode);
        lib.toggle_ai_mode();
        assert!(!lib.ai.as_ref().unwrap().book_mode);
        lib.toggle_ai_mode();
        assert!(lib.ai.as_ref().unwrap().book_mode);
    }

    #[test]
    fn test_ai_rows_book_mode_is_flat_books() {
        let lib = ai_ranked_library();
        let ai = lib.ai.as_ref().unwrap();
        assert_eq!(
            ai.rows(),
            vec![
                AiRow::Book { book_index: 0 },
                AiRow::Book { book_index: 1 },
                AiRow::Book { book_index: 2 },
            ]
        );
    }

    #[test]
    fn test_ai_rows_grouped_includes_headers_and_chapters() {
        let mut lib = ai_ranked_library();
        lib.toggle_ai_mode(); // grouped-chapter
        let ai = lib.ai.as_ref().unwrap();
        // Book 2 has one chapter; Books 1 and 0 have none.
        assert_eq!(
            ai.rows(),
            vec![
                AiRow::Book { book_index: 0 },
                AiRow::Chapter {
                    book_index: 0,
                    chapter_index: 0
                },
                AiRow::Book { book_index: 1 },
                AiRow::Book { book_index: 2 },
            ]
        );
    }

    #[test]
    fn test_ai_nav_moves_and_clamps() {
        let mut lib = ai_ranked_library();
        lib.ai_nav(1);
        assert_eq!(lib.ai.as_ref().unwrap().cursor, 1);
        lib.ai_nav(1);
        assert_eq!(lib.ai.as_ref().unwrap().cursor, 2);
        lib.ai_nav(1); // clamp at last
        assert_eq!(lib.ai.as_ref().unwrap().cursor, 2);
        lib.ai_nav(-1);
        assert_eq!(lib.ai.as_ref().unwrap().cursor, 1);
    }

    #[test]
    fn test_selected_book_follows_ai_cursor() {
        let mut lib = ai_ranked_library();
        assert_eq!(lib.selected_book().unwrap().title, "Book 2");
        lib.ai_nav(1);
        assert_eq!(lib.selected_book().unwrap().title, "Book 1");
    }

    #[test]
    fn test_remove_selected_preserves_ai_session() {
        let mut lib = ai_ranked_library();
        // Cursor on Book 2 (index 0) → removes that book, AI stays active.
        lib.remove_selected();
        assert!(lib.ai.is_some());
        assert_eq!(lib.books.len(), 2);
        // Derived order now skips the removed book.
        assert_eq!(lib.visible_indices(), vec![1, 0]);
    }

    #[test]
    fn test_remove_selected_removes_book_from_ai_session() {
        let mut lib = ai_ranked_library();
        lib.remove_selected();
        // The removed book is gone from the session too — no dangling row.
        let ai = lib.ai.as_ref().unwrap();
        assert!(!ai.results.books.iter().any(|b| b.book_url == "url-2"));
        assert_eq!(ai.results.books.len(), 2);
        // The cursor is clamped and the selection resolves to a real book.
        assert!(ai.cursor < ai.rows().len());
        assert!(lib.selected_book().is_some());
    }

    #[test]
    fn test_apply_ai_results_preserves_session_mode() {
        let mut lib = lib_with_n(2);
        lib.set_ai_searching("q".into());
        lib.toggle_ai_mode(); // grouped-chapter mode
        assert!(!lib.ai.as_ref().unwrap().book_mode);
        // A chapter-mode outcome must not reset the user's mode toggle.
        lib.apply_ai_results(
            "q".into(),
            SearchOutcome::ChapterMode {
                hits: vec![],
                embedded: 0,
                total: 0,
            },
        );
        assert!(!lib.ai.as_ref().unwrap().book_mode);
    }

    /// A library whose AI book-mode results carry genres across two books.
    fn genre_ranked_library() -> Library {
        let mut lib = lib_with_n(3);
        lib.apply_ai_results(
            "dragon".into(),
            SearchOutcome::BookMode {
                embedded: 3,
                total: 3,
                hits: vec![
                    AiBook {
                        book_url: "url-2".into(),
                        book_title: "Book 2".into(),
                        score: 90.0,
                        genres: vec!["Fantasy".into(), "LitRPG".into()],
                        chapters: vec![],
                    },
                    AiBook {
                        book_url: "url-1".into(),
                        book_title: "Book 1".into(),
                        score: 70.0,
                        genres: vec!["Fantasy".into()],
                        chapters: vec![],
                    },
                    AiBook {
                        book_url: "url-0".into(),
                        book_title: "Book 0".into(),
                        score: 50.0,
                        genres: vec!["Sci-Fi".into()],
                        chapters: vec![],
                    },
                ],
            },
        );
        lib
    }

    #[test]
    fn test_new_ai_session_starts_with_no_genre() {
        let lib = genre_ranked_library();
        assert!(lib.ai.as_ref().unwrap().genre.is_none());
    }

    #[test]
    fn test_cycle_ai_genre_advances_and_wraps_to_none() {
        let mut lib = genre_ranked_library();
        // None → Fantasy (sorted: Fantasy, LitRPG, Sci-Fi).
        lib.cycle_ai_genre(1);
        assert_eq!(lib.ai.as_ref().unwrap().genre.as_deref(), Some("Fantasy"));
        lib.cycle_ai_genre(1);
        assert_eq!(lib.ai.as_ref().unwrap().genre.as_deref(), Some("LitRPG"));
        lib.cycle_ai_genre(1);
        assert_eq!(lib.ai.as_ref().unwrap().genre.as_deref(), Some("Sci-Fi"));
        // Wraps back to none.
        lib.cycle_ai_genre(1);
        assert!(lib.ai.as_ref().unwrap().genre.is_none());
    }

    #[test]
    fn test_cycle_ai_genre_retreats_and_wraps() {
        let mut lib = genre_ranked_library();
        // None → Sci-Fi (retreat wraps to the last genre).
        lib.cycle_ai_genre(-1);
        assert_eq!(lib.ai.as_ref().unwrap().genre.as_deref(), Some("Sci-Fi"));
        lib.cycle_ai_genre(-1);
        assert_eq!(lib.ai.as_ref().unwrap().genre.as_deref(), Some("LitRPG"));
    }

    #[test]
    fn test_cycle_ai_genre_filters_derived_order() {
        let mut lib = genre_ranked_library();
        lib.cycle_ai_genre(1); // Fantasy → books 2 and 1 (url-2, url-1).
        assert_eq!(lib.visible_indices(), vec![2, 1]);
        lib.cycle_ai_genre(1); // LitRPG → only book 2.
        assert_eq!(lib.visible_indices(), vec![2]);
        lib.cycle_ai_genre(1); // Sci-Fi → only book 0.
        assert_eq!(lib.visible_indices(), vec![0]);
    }

    #[test]
    fn test_cycle_ai_genre_no_genres_is_noop() {
        let mut lib = ai_ranked_library(); // all books carry empty genres
        lib.cycle_ai_genre(1);
        assert!(lib.ai.as_ref().unwrap().genre.is_none());
        assert_eq!(lib.visible_indices(), vec![2, 1, 0]);
    }

    #[test]
    fn test_cycle_ai_genre_resets_cursor() {
        let mut lib = genre_ranked_library();
        lib.ai_nav(1);
        lib.ai_nav(1); // cursor moves off the first row
        assert_eq!(lib.ai.as_ref().unwrap().cursor, 2);
        lib.cycle_ai_genre(1);
        assert_eq!(lib.ai.as_ref().unwrap().cursor, 0);
    }

    #[test]
    fn test_new_search_resets_genre() {
        let mut lib = genre_ranked_library();
        lib.cycle_ai_genre(1); // Fantasy active
        assert_eq!(lib.ai.as_ref().unwrap().genre.as_deref(), Some("Fantasy"));
        // A fresh search (refine) resets the facet.
        lib.set_ai_searching("dragon heart".into());
        assert!(lib.ai.as_ref().unwrap().genre.is_none());
        assert_eq!(lib.ai.as_ref().unwrap().visible_book_count(), 0);
    }

    #[test]
    fn test_clear_ai_clears_genre() {
        let mut lib = genre_ranked_library();
        lib.cycle_ai_genre(1);
        lib.clear_ai();
        assert!(lib.ai.is_none());
        assert_eq!(lib.visible_indices(), vec![0, 1, 2]);
    }

    #[test]
    fn test_genre_filtered_rows_and_book_count() {
        let lib = genre_ranked_library();
        let ai = lib.ai.as_ref().unwrap();
        assert_eq!(ai.visible_book_count(), 3);
        assert_eq!(
            ai.rows(),
            vec![
                AiRow::Book { book_index: 0 },
                AiRow::Book { book_index: 1 },
                AiRow::Book { book_index: 2 },
            ]
        );
    }
}
