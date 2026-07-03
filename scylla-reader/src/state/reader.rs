//! Reader state — content, paging, scrolling, word-wrap calculations.

pub struct ReaderState {
    pub content: Vec<String>,
    pub scroll: usize,
    pub page: usize,
    pub chapter_title: String,
    pub book_title: String,
    pub book_url: String,
    pub current_chapter_idx: usize,
    pub loading: bool,
}

impl ReaderState {
    pub fn new() -> Self {
        Self {
            content: Vec::new(),
            scroll: 0,
            page: 0,
            chapter_title: String::new(),
            book_title: String::new(),
            book_url: String::new(),
            current_chapter_idx: 0,
            loading: false,
        }
    }

    pub fn load(
        &mut self,
        book_title: String,
        book_url: String,
        chapter_title: String,
        content: String,
        chapter_idx: usize,
    ) {
        self.book_title = book_title;
        self.book_url = book_url;
        self.chapter_title = chapter_title;
        let mut lines: Vec<String> = content.lines().map(|l| l.to_string()).collect();
        while let Some(last) = lines.last() {
            if last.trim().is_empty() {
                lines.pop();
            } else {
                break;
            }
        }
        self.content = lines;
        self.scroll = 0;
        self.page = 0;
        self.current_chapter_idx = chapter_idx;
        self.loading = false;
    }

    pub fn lines_per_page(area_height: u16) -> usize {
        area_height.saturating_sub(4) as usize
    }

    pub fn total_pages_for(&self, area_width: u16, area_height: u16) -> usize {
        let lpp = Self::lines_per_page(area_height);
        if lpp == 0 {
            return 1;
        }
        let width = area_width as usize;
        let visual_lines: usize = self
            .content
            .iter()
            .map(|l| Self::wrap_line_count(l, width))
            .sum();
        let pages = (visual_lines + lpp - 1) / lpp;
        if pages == 0 { 1 } else { pages }
    }

    pub fn total_pages(&self, area_height: u16) -> usize {
        self.total_pages_for(80, area_height)
    }

    pub fn page_lines_wrapped(&self, area_width: u16, area_height: u16) -> Vec<String> {
        let lpp = Self::lines_per_page(area_height);
        if lpp == 0 {
            return vec![];
        }
        let width = area_width as usize;
        let mut vlines: Vec<String> = Vec::new();
        for line in &self.content {
            let parts = Self::wrap_line(line, width);
            if parts.is_empty() {
                vlines.push(String::new());
            } else {
                vlines.extend(parts);
            }
        }
        if vlines.is_empty() {
            vlines.push(String::new());
        }
        let total_pages = (vlines.len() + lpp - 1) / lpp;
        let page_idx = std::cmp::min(self.page, total_pages.saturating_sub(1));
        let start = page_idx * lpp;
        let end = (start + lpp).min(vlines.len());
        vlines[start..end].to_vec()
    }

    fn wrap_line_count(line: &str, width: usize) -> usize {
        if width == 0 {
            return 1;
        }
        if line.trim().is_empty() {
            return 1;
        }
        let mut count = 0usize;
        let mut cur = 0usize;
        for word in line.split_whitespace() {
            let wlen = word.chars().count();
            if cur == 0 {
                cur = wlen;
            } else if cur + 1 + wlen <= width {
                cur += 1 + wlen;
            } else {
                count += 1;
                cur = wlen;
            }
        }
        if cur > 0 {
            count += 1;
        }
        count
    }

    fn wrap_line(line: &str, width: usize) -> Vec<String> {
        if width == 0 {
            return vec![line.to_string()];
        }
        if line.trim().is_empty() {
            return vec![String::new()];
        }
        let mut parts: Vec<String> = Vec::new();
        let mut cur = String::new();
        let mut cur_len = 0usize;
        for word in line.split_whitespace() {
            let wlen = word.chars().count();
            if cur.is_empty() {
                cur.push_str(word);
                cur_len = wlen;
            } else if cur_len + 1 + wlen <= width {
                cur.push(' ');
                cur.push_str(word);
                cur_len += 1 + wlen;
            } else {
                parts.push(cur);
                cur = word.to_string();
                cur_len = wlen;
            }
        }
        if !cur.is_empty() {
            parts.push(cur);
        }
        parts
    }

    pub fn next_page(&mut self, area_width: u16, area_height: u16) {
        if self.page + 1 < self.total_pages_for(area_width, area_height) {
            self.page += 1;
        }
    }

    pub fn prev_page(&mut self, _area_width: u16, _area_height: u16) {
        if self.page > 0 {
            self.page -= 1;
        }
    }

    pub fn scroll_down(&mut self, amount: usize) {
        if self.content.is_empty() {
            return;
        }
        self.scroll = (self.scroll + amount).min(self.content.len().saturating_sub(1));
    }

    pub fn scroll_up(&mut self, amount: usize) {
        self.scroll = self.scroll.saturating_sub(amount);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_state_is_empty() {
        let r = ReaderState::new();
        assert!(r.content.is_empty());
        assert_eq!(r.scroll, 0);
        assert_eq!(r.page, 0);
        assert_eq!(r.current_chapter_idx, 0);
        assert!(!r.loading);
    }

    #[test]
    fn test_load_basic() {
        let mut r = ReaderState::new();
        r.load("Book".into(), "url".into(), "Ch1".into(), "hello\nworld\n".into(), 0);
        assert_eq!(r.content, vec!["hello", "world"]);
        assert_eq!(r.book_title, "Book");
        assert_eq!(r.chapter_title, "Ch1");
        assert_eq!(r.current_chapter_idx, 0);
        assert!(!r.loading);
    }

    #[test]
    fn test_load_strips_trailing_blanks() {
        let mut r = ReaderState::new();
        r.load("".into(), "".into(), "".into(), "a\nb\n\n  \n\t\n".into(), 0);
        assert_eq!(r.content, vec!["a", "b"]);
    }

    #[test]
    fn test_load_preserves_internal_blanks() {
        let mut r = ReaderState::new();
        r.load("".into(), "".into(), "".into(), "a\n\n\nb".into(), 0);
        assert_eq!(r.content, vec!["a", "", "", "b"]);
    }

    #[test]
    fn test_load_empty_content() {
        let mut r = ReaderState::new();
        r.load("".into(), "".into(), "".into(), "".into(), 0);
        assert!(r.content.is_empty());
    }

    #[test]
    fn test_lines_per_page_normal() {
        assert_eq!(ReaderState::lines_per_page(20), 16);
    }

    #[test]
    fn test_lines_per_page_small() {
        assert_eq!(ReaderState::lines_per_page(2), 0);
    }

    #[test]
    fn test_total_pages_for_empty_content() {
        let r = ReaderState::new();
        assert_eq!(r.total_pages_for(80, 20), 1);
    }

    #[test]
    fn test_total_pages_for_single_line() {
        let mut r = ReaderState::new();
        r.load("".into(), "".into(), "".into(), "hello".into(), 0);
        assert_eq!(r.total_pages_for(80, 20), 1);
    }

    #[test]
    fn test_total_pages_for_multi_page() {
        let mut r = ReaderState::new();
        let content = (0..50).map(|i| format!("line {}", i)).collect::<Vec<_>>().join("\n");
        r.load("".into(), "".into(), "".into(), content, 0);
        assert_eq!(r.total_pages_for(80, 20), 4);
    }

    #[test]
    fn test_wrap_line_count_zero_width() {
        assert_eq!(ReaderState::wrap_line_count("hello world", 0), 1);
    }

    #[test]
    fn test_wrap_line_count_empty_line() {
        assert_eq!(ReaderState::wrap_line_count("   ", 80), 1);
    }

    #[test]
    fn test_wrap_line_count_fits() {
        assert_eq!(ReaderState::wrap_line_count("hello", 80), 1);
    }

    #[test]
    fn test_wrap_line_count_wraps() {
        let line = "word1 word2 word3 word4 word5";
        assert_eq!(ReaderState::wrap_line_count(line, 10), 5);
    }

    #[test]
    fn test_wrap_line_count_very_long_word() {
        let long_word = "a".repeat(100);
        assert_eq!(ReaderState::wrap_line_count(&long_word, 10), 1);
    }

    #[test]
    fn test_wrap_line_zero_width() {
        let result = ReaderState::wrap_line("hello world", 0);
        assert_eq!(result, vec!["hello world"]);
    }

    #[test]
    fn test_wrap_line_blank_line() {
        let result = ReaderState::wrap_line("   ", 80);
        assert_eq!(result, vec![""]);
    }

    #[test]
    fn test_wrap_line_fits() {
        let result = ReaderState::wrap_line("hello world", 80);
        assert_eq!(result, vec!["hello world"]);
    }

    #[test]
    fn test_wrap_line_splits() {
        let result = ReaderState::wrap_line("a b c d e", 3);
        assert_eq!(result, vec!["a b", "c d", "e"]);
    }

    #[test]
    fn test_wrap_line_single_word_fits() {
        let result = ReaderState::wrap_line("hello", 10);
        assert_eq!(result, vec!["hello"]);
    }

    #[test]
    fn test_page_lines_wrapped_empty() {
        let r = ReaderState::new();
        let lines = r.page_lines_wrapped(80, 20);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].is_empty());
    }

    #[test]
    fn test_page_lines_wrapped_clamps_page() {
        let mut r = ReaderState::new();
        r.load("".into(), "".into(), "".into(), "line1\nline2".into(), 0);
        r.page = 99;
        let lines = r.page_lines_wrapped(80, 20);
        assert!(!lines.is_empty());
    }

    #[test]
    fn test_page_lines_wrapped_returns_correct_page() {
        let mut r = ReaderState::new();
        let content = (0..10).map(|i| format!("line {}", i)).collect::<Vec<_>>().join("\n");
        r.load("".into(), "".into(), "".into(), content, 0);
        r.page = 0;
        let page0 = r.page_lines_wrapped(80, 6);
        assert_eq!(page0.len(), 2);
        assert!(page0[0].contains("line 0"));
    }

    #[test]
    fn test_next_page_increments() {
        let mut r = ReaderState::new();
        r.load("".into(), "".into(), "".into(), (0..10).map(|i| format!("line {}", i)).collect::<Vec<_>>().join("\n"), 0);
        r.next_page(80, 6);
        assert_eq!(r.page, 1);
    }

    #[test]
    fn test_next_page_clamps() {
        let mut r = ReaderState::new();
        r.load("".into(), "".into(), "".into(), "single line".into(), 0);
        r.next_page(80, 20);
        assert_eq!(r.page, 0);
    }

    #[test]
    fn test_prev_page_decrements() {
        let mut r = ReaderState::new();
        r.load("".into(), "".into(), "".into(), (0..10).map(|i| format!("line {}", i)).collect::<Vec<_>>().join("\n"), 0);
        r.page = 2;
        r.prev_page(80, 6);
        assert_eq!(r.page, 1);
    }

    #[test]
    fn test_prev_page_clamps() {
        let mut r = ReaderState::new();
        r.load("".into(), "".into(), "".into(), "single line".into(), 0);
        r.prev_page(80, 20);
        assert_eq!(r.page, 0);
    }

    #[test]
    fn test_scroll_down_normal() {
        let mut r = ReaderState::new();
        r.load("".into(), "".into(), "".into(), (0..10).map(|i| format!("line {}", i)).collect::<Vec<_>>().join("\n"), 0);
        r.scroll_down(3);
        assert_eq!(r.scroll, 3);
    }

    #[test]
    fn test_scroll_down_clamps() {
        let mut r = ReaderState::new();
        r.load("".into(), "".into(), "".into(), (0..3).map(|i| format!("line {}", i)).collect::<Vec<_>>().join("\n"), 0);
        r.scroll_down(100);
        assert_eq!(r.scroll, 2);
    }

    #[test]
    fn test_scroll_down_empty_content_does_nothing() {
        let mut r = ReaderState::new();
        r.scroll_down(5);
        assert_eq!(r.scroll, 0);
    }

    #[test]
    fn test_scroll_up_normal() {
        let mut r = ReaderState::new();
        r.load("".into(), "".into(), "".into(), (0..10).map(|i| format!("line {}", i)).collect::<Vec<_>>().join("\n"), 0);
        r.scroll = 5;
        r.scroll_up(2);
        assert_eq!(r.scroll, 3);
    }

    #[test]
    fn test_scroll_up_clamps() {
        let mut r = ReaderState::new();
        r.scroll = 3;
        r.scroll_up(10);
        assert_eq!(r.scroll, 0);
    }

    #[test]
    fn test_total_pages_delegates() {
        let mut r = ReaderState::new();
        r.load("".into(), "".into(), "".into(), (0..50).map(|i| format!("line {}", i)).collect::<Vec<_>>().join("\n"), 0);
        assert_eq!(r.total_pages(20), 4);
    }

    #[test]
    fn test_wrap_line_count_matches_actual_wrapped_lines() {
        let line = "this is a test line with several words";
        let width = 10;
        let count = ReaderState::wrap_line_count(line, width);
        let wrapped = ReaderState::wrap_line(line, width);
        assert_eq!(count, wrapped.len());
    }
}
