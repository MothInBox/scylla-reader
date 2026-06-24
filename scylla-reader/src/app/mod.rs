pub mod page;
pub use page::Page;

use crate::library::{self, Library};
use crate::models::Book;
use crate::models::Chapter;
use crate::settings::Settings;
use std::fmt;

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

    pub fn prev_page(&mut self, area_width: u16, area_height: u16) {
        if self.page > 0 {
            self.page -= 1;
        }
    }

    pub fn scroll_down(&mut self, amount: usize) {
        self.scroll = (self.scroll + amount).min(self.content.len().saturating_sub(1));
    }

    pub fn scroll_up(&mut self, amount: usize) {
        self.scroll = self.scroll.saturating_sub(amount);
    }
}

pub enum WinInput {
    RawText(String),
    ChapterItem(Chapter),
}
impl WinInput {
    pub fn is_empty(&self) -> bool {
        match self {
            WinInput::RawText(s) => s.is_empty(),
            WinInput::ChapterItem(c) => c.title.is_empty(),
        }
    }

    pub fn pop(&mut self) -> Option<char> {
        match self {
            WinInput::RawText(s) => s.pop(),
            WinInput::ChapterItem(_) => None, // Chapters are usually immutable in UI input fields
        }
    }

    pub fn push(&mut self, c: char) {
        match self {
            WinInput::RawText(s) => s.push(c),
            WinInput::ChapterItem(_) => {} // Ignore typing characters if it's a structural Chapter
        }
    }

    pub fn trim_to_string(&self) -> String {
        match self {
            WinInput::RawText(s) => s.trim().to_string(),
            WinInput::ChapterItem(c) => c.title.trim().to_string(),
        }
    }
}
impl fmt::Display for WinInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WinInput::RawText(s) => write!(f, "{}", s),
            WinInput::ChapterItem(c) => write!(f, "{}", c.title), // Assuming it has .title
        }
    }
}

pub struct AppState {
    pub library: Library,
    pub current_page: Page,
    pub win_inputs: Vec<WinInput>,
    pub win_cursor: usize,
    pub win_scroll_offset: usize,
    pub settings: Settings,
    pub reader: ReaderState,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            library: Library::new(),
            current_page: Page::Library,
            win_inputs: vec![WinInput::RawText(String::new())],
            win_cursor: 0,
            win_scroll_offset: 0,
            settings: Settings::new(),
            reader: ReaderState::new(),
        }
    }

    pub fn reset_win_input(&mut self) {
        self.win_inputs = vec![WinInput::RawText(String::new())];
        self.win_cursor = 0;
        self.win_scroll_offset = 0;
    }
    pub fn current_line_mut(&mut self) -> &mut WinInput {
        &mut self.win_inputs[self.win_cursor]
    }

    pub fn valid_win_input(&self) -> Vec<WinInput> {
        self.win_inputs
            .iter()
            .map(|s| s.trim_to_string())
            .filter(|s| !s.is_empty())
            .map(WinInput::RawText)
            .collect()
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
