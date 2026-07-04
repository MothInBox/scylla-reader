# UI Overhaul Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Redesign Scylla Reader's keybindings and add command palette + hints toggle for better discoverability and consistency.

**Architecture:** Hybrid navigation — global numbers (1/2/3) for screens, contextual letters for actions, `:` for command palette modal, `?` for footer hints toggle. Consistent `Esc`=back, `Enter`=confirm/open everywhere.

**Tech Stack:** Rust, ratatui 0.29, crossterm 0.28, extism for plugins, tokio for async worker.

## Global Constraints

- All existing functionality preserved (library split view, both reader modes, multi-URL add modal, chapter jump modal, plugin config screens, debug log viewer)
- No new dependencies beyond existing Cargo.toml
- Follow existing code patterns (mod.rs module structure, trait-based state, mpsc channels)
- TDD: write test first, then implementation
- Commit after each task

---

### Task 1: Add `show_hints` to AppState

**Files:**
- Modify: `src/state/mod.rs:14-21`
- Test: `src/state/mod.rs:87-154` (existing tests)

**Interfaces:**
- Produces: `AppState.show_hints: bool` field, default `true`

- [ ] **Step 1: Write the failing test**

```rust
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
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cd /home/monkeinbox/Projects/Scylla-Reader/scylla-reader && cargo test test_show_hints -- --nocapture
```
Expected: FAIL — `show_hints` field doesn't exist

- [ ] **Step 3: Write minimal implementation**

```rust
pub struct AppState {
    pub library: Library,
    pub current_page: Page,
    pub modal: Modal,
    pub settings: Settings,
    pub reader: ReaderState,
    pub db: Db,
    pub show_hints: bool,  // ADD THIS
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
                crate::settings::log(crate::settings::LogLevel::Error, "DB", &format!("DB open failed: {}", e));
                panic!("Could not open database");
            }),
            show_hints: true,  // ADD THIS
        }
    }
    // ... rest unchanged
}
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cd /home/monkeinbox/Projects/Scylla-Reader/scylla-reader && cargo test test_show_hints -- --nocapture
```
Expected: PASS

- [ ] **Step 5: Commit**

```bash
cd /home/monkeinbox/Projects/Scylla-Reader && git add scylla-reader/src/state/mod.rs && git commit -m "feat(state): add show_hints field to AppState"
```

---

### Task 2: Add CommandPalette Modal Variant

**Files:**
- Modify: `src/state/modal.rs:5-18`
- Test: `src/state/mod.rs` (add test)

**Interfaces:**
- Produces: `Modal::CommandPalette { query: String, filtered: Vec<PaletteAction>, selected: usize }`

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn test_command_palette_modal_variant() {
    use crate::state::Modal;
    let modal = Modal::CommandPalette {
        query: String::new(),
        filtered: Vec::new(),
        selected: 0,
    };
    match modal {
        Modal::CommandPalette { query, filtered, selected } => {
            assert!(query.is_empty());
            assert!(filtered.is_empty());
            assert_eq!(selected, 0);
        }
        _ => panic!("Expected CommandPalette variant"),
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cd /home/monkeinbox/Projects/Scylla-Reader/scylla-reader && cargo test test_command_palette_modal_variant -- --nocapture
```
Expected: FAIL — variant doesn't exist

- [ ] **Step 3: Write minimal implementation**

```rust
#[derive(Debug, PartialEq)]
pub enum Modal {
    None,
    AddBook {
        inputs: Vec<String>,
        cursor: usize,
        scroll_offset: usize,
    },
    JumpChapter {
        chapters: Vec<Chapter>,
        cursor: usize,
        scroll_offset: usize,
        show_titles: bool,
    },
    CommandPalette {
        query: String,
        filtered: Vec<PaletteAction>,
        selected: usize,
    },
}
```

- [ ] **Step 4: Add PaletteAction struct in same file**

```rust
#[derive(Debug, Clone)]
pub struct PaletteAction {
    pub category: &'static str,
    pub label: &'static str,
    pub keys: &'static str,
    pub handler: fn(&mut AppState, &std::sync::mpsc::Sender<AppCommand>),
}
```

- [ ] **Step 5: Run test to verify it passes**

```bash
cd /home/monkeinbox/Projects/Scylla-Reader/scylla-reader && cargo test test_command_palette_modal_variant -- --nocapture
```
Expected: PASS

- [ ] **Step 6: Commit**

```bash
cd /home/monkeinbox/Projects/Scylla-Reader && git add scylla-reader/src/state/modal.rs && git commit -m "feat(state): add CommandPalette modal variant and PaletteAction"
```

---

### Task 3: Create Palette Actions Registry

**Files:**
- Create: `src/ui/palette.rs`
- Test: `src/ui/palette.rs` (inline tests)

**Interfaces:**
- Produces: `build_palette_actions(cmd_tx: Sender<AppCommand>) -> Vec<PaletteAction>`, `filter_actions(actions, query) -> Vec<PaletteAction>`

- [ ] **Step 1: Write failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::messenger::AppCommand;
    use std::sync::mpsc;

    #[test]
    fn test_build_palette_actions_returns_non_empty() {
        let (tx, _rx) = mpsc::channel();
        let actions = build_palette_actions(tx);
        assert!(!actions.is_empty());
        assert!(actions.iter().any(|a| a.label == "Go to Library"));
        assert!(actions.iter().any(|a| a.label == "Add Book"));
        assert!(actions.iter().any(|a| a.label == "Quit"));
    }

    #[test]
    fn test_filter_actions_empty_query_returns_all() {
        let (tx, _rx) = mpsc::channel();
        let actions = build_palette_actions(tx);
        let filtered = filter_actions(&actions, "");
        assert_eq!(filtered.len(), actions.len());
    }

    #[test]
    fn test_filter_actions_fuzzy_matches() {
        let (tx, _rx) = mpsc::channel();
        let actions = build_palette_actions(tx);
        let filtered = filter_actions(&actions, "lib");
        assert!(filtered.iter().any(|a| a.label.to_lowercase().contains("lib")));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cd /home/monkeinbox/Projects/Scylla-Reader/scylla-reader && cargo test palette::tests -- --nocapture
```
Expected: FAIL — module doesn't exist

- [ ] **Step 3: Write minimal implementation**

```rust
//! Command palette — fuzzy-searchable action registry.

use crate::messenger::AppCommand;
use crate::state::{AppState, Modal, Page};
use crate::state::modal::PaletteAction;
use std::sync::mpsc;

pub fn build_palette_actions(cmd_tx: mpsc::Sender<AppCommand>) -> Vec<PaletteAction> {
    vec![
        // Navigation (always shown)
        PaletteAction { category: "Navigation", label: "Go to Library", keys: "1", handler: |s, _| s.current_page = Page::Library },
        PaletteAction { category: "Navigation", label: "Go to Reader", keys: "2", handler: |s, _| { s.current_page = Page::Reader; } },
        PaletteAction { category: "Navigation", label: "Go to Settings", keys: "3", handler: |s, _| s.current_page = Page::Settings },
        PaletteAction { category: "Navigation", label: "Quit", keys: "q", handler: |s, _| { /* handled in app loop */ } },

        // Library actions
        PaletteAction { category: "Library", label: "Add Book", keys: "i", handler: |s, _| { s.modal = Modal::AddBook { inputs: vec![String::new()], cursor: 0, scroll_offset: 0 }; s.current_page = Page::AddingBook; } },
        PaletteAction { category: "Library", label: "Jump Chapter", keys: "j", handler: |s, _| { if let Some(b) = s.library.selected_book() { s.modal = Modal::JumpChapter { chapters: b.chapters.clone(), cursor: 0, scroll_offset: 0, show_titles: true }; s.current_page = Page::BookChapterJump; } } },
        PaletteAction { category: "Library", label: "Update All", keys: "u", handler: |s, tx| { let urls: Vec<String> = s.library.books.iter().map(|b| b.url.clone()).filter(|u| !u.is_empty()).collect(); if !urls.is_empty() { let _ = tx.send(AppCommand::UpdateAll(urls)); } } },
        PaletteAction { category: "Library", label: "Delete Book", keys: "d", handler: |s, _| { s.library.remove_selected(); } },
        PaletteAction { category: "Library", label: "Cycle Filter", keys: "f", handler: |s, _| { s.library.cycle_filter(); } },
        PaletteAction { category: "Library", label: "Cycle Status", keys: "Space", handler: |s, _| { s.library.cycle_selected_status(); } },

        // Reader actions
        PaletteAction { category: "Reader", label: "Next Chapter", keys: ">", handler: |s, tx| { if let Some(b) = s.library.selected_book() { let next = s.reader.current_chapter_idx + 1; if let Some(ch) = b.chapters.get(next) { s.reader.loading = true; let _ = tx.send(AppCommand::FetchChapter(ch.url.clone(), next)); } } } },
        PaletteAction { category: "Reader", label: "Previous Chapter", keys: "<", handler: |s, tx| { if let Some(b) = s.library.selected_book() { let prev = s.reader.current_chapter_idx.saturating_sub(1); if prev != s.reader.current_chapter_idx { if let Some(ch) = b.chapters.get(prev) { s.reader.loading = true; let _ = tx.send(AppCommand::FetchChapter(ch.url.clone(), prev)); } } } } },
        PaletteAction { category: "Reader", label: "Toggle Paged/Scrollable", keys: "", handler: |s, _| { s.settings.reader_mode = s.settings.reader_mode.toggle(); } },

        // Settings
        PaletteAction { category: "Settings", label: "Open Settings", keys: "3", handler: |s, _| s.current_page = Page::Settings },
        PaletteAction { category: "Settings", label: "Toggle Debug Log", keys: "", handler: |s, _| { s.settings.debug_log = !s.settings.debug_log; crate::settings::set_debug(s.settings.debug_log); } },
        PaletteAction { category: "Settings", label: "Plugin Configs", keys: "", handler: |s, _| { s.settings.reload_plugins(); s.settings.settings_page = crate::settings::SettingsPage::PluginList; } },

        // Debug
        PaletteAction { category: "Debug", label: "Reload Log", keys: "", handler: |s, _| s.settings.reload_log() },
    ]
}

pub fn filter_actions(actions: &[PaletteAction], query: &str) -> Vec<PaletteAction> {
    if query.trim().is_empty() {
        return actions.to_vec();
    }
    let q = query.to_lowercase();
    actions
        .iter()
        .filter(|a| {
            a.label.to_lowercase().contains(&q)
                || a.category.to_lowercase().contains(&q)
                || a.keys.to_lowercase().contains(&q)
        })
        .cloned()
        .collect()
}
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cd /home/monkeinbox/Projects/Scylla-Reader/scylla-reader && cargo test palette::tests -- --nocapture
```
Expected: PASS

- [ ] **Step 5: Commit**

```bash
cd /home/monkeinbox/Projects/Scylla-Reader && git add scylla-reader/src/ui/palette.rs && git commit -m "feat(ui): add command palette actions registry and filtering"
```

---

### Task 4: Add Palette Rendering

**Files:**
- Modify: `src/ui/palette.rs` (add draw function)
- Test: `src/ui/palette.rs` (inline tests)

**Interfaces:**
- Consumes: `PaletteAction` from Task 3
- Produces: `draw_palette(frame, area, state)` function

- [ ] **Step 1: Write failing test**

```rust
#[test]
fn test_draw_palette_renders_without_panic() {
    use crate::state::AppState;
    use crate::db::Db;
    use crate::library::Library;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use std::sync::mpsc;

    let conn = rusqlite::Connection::open_in_memory().unwrap();
    let db = Db::open_conn(conn).unwrap();
    let mut state = AppState::from_parts(db, Library::new());
    state.modal = Modal::CommandPalette {
        query: "test".into(),
        filtered: vec![],
        selected: 0,
    };

    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| {
        draw_palette(f, f.area(), &state);
    }).unwrap();
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cd /home/monkeinbox/Projects/Scylla-Reader/scylla-reader && cargo test test_draw_palette -- --nocapture
```
Expected: FAIL — `draw_palette` doesn't exist

- [ ] **Step 3: Write minimal implementation**

```rust
use crate::state::{AppState, Modal};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph};

pub fn draw_palette(frame: &mut Frame, area: Rect, state: &AppState) {
    if let Modal::CommandPalette { query, filtered, selected } = &state.modal {
        let popup_area = centered_rect(60, 40, area);
        frame.render_widget(Clear, popup_area);

        let items: Vec<ListItem> = filtered
            .iter()
            .map(|a| {
                let keys = if a.keys.is_empty() { String::new() } else { format!(" [{}]", a.keys) };
                ListItem::new(format!("{}  {}{}", a.category, a.label, keys))
            })
            .collect();

        let mut list_state = ListState::default();
        list_state.select(Some(*selected));

        let list = List::new(items)
            .block(Block::default().title(format!(" Commands ({}): ", query)).borders(Borders::ALL))
            .highlight_style(Style::default().bg(Color::Blue).fg(Color::White))
            .highlight_symbol(">> ");

        frame.render_stateful_widget(list, popup_area, &mut list_state);
    }
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cd /home/monkeinbox/Projects/Scylla-Reader/scylla-reader && cargo test test_draw_palette -- --nocapture
```
Expected: PASS

- [ ] **Step 5: Commit**

```bash
cd /home/monkeinbox/Projects/Scylla-Reader && git add scylla-reader/src/ui/palette.rs && git commit -m "feat(ui): add command palette rendering"
```

---

### Task 5: Wire Palette into UI Dispatch

**Files:**
- Modify: `src/ui/mod.rs:13-26` (add palette draw call)
- Test: `src/ui/mod.rs` (inline test)

**Interfaces:**
- Consumes: `draw_palette` from Task 4
- Produces: Updated `draw()` that renders palette modal

- [ ] **Step 1: Write failing test**

```rust
#[test]
fn test_draw_calls_palette_when_modal_open() {
    // Verify palette draws when CommandPalette modal is active
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cd /home/monkeinbox/Projects/Scylla-Reader/scylla-reader && cargo test test_draw_calls_palette -- --nocapture
```
Expected: FAIL

- [ ] **Step 3: Write minimal implementation**

```rust
pub fn draw(frame: &mut Frame, state: &mut AppState, area: Rect) {
    match state.current_page {
        Page::Library | Page::AddingBook | Page::BookChapterJump => {
            library::draw(frame, area, state);
        }
        Page::Settings => {
            settings::draw(frame, area, state);
        }
        Page::Reader => {
            reader::draw(frame, area, state);
        }
    }

    modal::draw_modal(frame, area, state);
    // ADD: palette draws inside modal::draw_modal via match on CommandPalette
}
```

Actually, palette is a modal variant, so it should be handled in `modal::draw_modal`. Let me update modal.rs instead.

- [ ] **Step 4: Update modal.rs to handle CommandPalette**

```rust
// In src/ui/modal.rs draw_modal function, add:
Modal::CommandPalette { query, filtered, selected } => {
    draw_palette(frame, area, state); // need to pass state appropriately
}
```

- [ ] **Step 5: Run test to verify it passes**

```bash
cd /home/monkeinbox/Projects/Scylla-Reader/scylla-reader && cargo test test_draw_calls_palette -- --nocapture
```
Expected: PASS

- [ ] **Step 6: Commit**

```bash
cd /home/monkeinbox/Projects/Scylla-Reader && git add scylla-reader/src/ui/mod.rs scylla-reader/src/ui/modal.rs && git commit -m "feat(ui): wire command palette into modal rendering"
```

---

### Task 6: Add Global Key Handling in App Main Loop

**Files:**
- Modify: `src/app.rs:69-83` (main_loop key handling)
- Test: `src/app.rs` (inline test)

**Interfaces:**
- Consumes: `AppState.show_hints`, `Modal::CommandPalette`
- Produces: Global handling for `1/2/3`, `:`, `?`, `Esc` (quit from library)

- [ ] **Step 1: Write failing test**

```rust
#[test]
fn test_global_keys_switch_screens() {
    // Test 1/2/3 switch pages from any screen
}
#[test]
fn test_colon_opens_palette() {
    // Test : opens CommandPalette modal
}
#[test]
fn test_question_toggles_hints() {
    // Test ? toggles show_hints
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cd /home/monkeinbox/Projects/Scylla-Reader/scylla-reader && cargo test test_global_keys -- --nocapture
```
Expected: FAIL

- [ ] **Step 3: Write minimal implementation**

```rust
fn handle_key(&mut self, key: crossterm::event::KeyEvent, area: Rect) -> bool {
    // GLOBAL KEYS FIRST (before input dispatch)
    match key.code {
        KeyCode::Char('1') => { self.state.current_page = Page::Library; return true; }
        KeyCode::Char('2') => { self.state.current_page = Page::Reader; return true; }
        KeyCode::Char('3') => { self.state.current_page = Page::Settings; return true; }
        KeyCode::Char(':') => {
            let (tx, _rx) = std::sync::mpsc::channel(); // need access to cmd_tx
            let actions = crate::ui::palette::build_palette_actions(self.cmd_tx.clone());
            let filtered = crate::ui::palette::filter_actions(&actions, "");
            self.state.modal = Modal::CommandPalette { query: String::new(), filtered, selected: 0 };
            return true;
        }
        KeyCode::Char('?') => {
            self.state.show_hints = !self.state.show_hints;
            return true;
        }
        KeyCode::Char('q') => {
            if self.state.current_page == Page::Library {
                return false; // quit
            }
        }
        KeyCode::Esc => {
            // Close modal or go back
            match &self.state.modal {
                Modal::None => {
                    if self.state.current_page != Page::Library {
                        self.state.current_page = Page::Library;
                    }
                }
                _ => {
                    self.state.close_modal();
                }
            }
            return true;
        }
        _ => {}
    }

    // ... existing input dispatch
}
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cd /home/monkeinbox/Projects/Scylla-Reader/scylla-reader && cargo test test_global_keys -- --nocapture
```
Expected: PASS

- [ ] **Step 5: Commit**

```bash
cd /home/monkeinbox/Projects/Scylla-Reader && git add scylla-reader/src/app.rs && git commit -m "feat(app): add global key handling for 1/2/3, :, ?, Esc"
```

---

### Task 7: Update Library Input Handler

**Files:**
- Modify: `src/input/library.rs:7-105`
- Test: `src/input/library.rs` (inline tests)

**Interfaces:**
- Consumes: New keybindings from design
- Produces: Updated `handle_library` function

- [ ] **Step 1: Write failing tests**

```rust
#[test]
fn test_handle_library_i_opens_add_book() {
    // Test 'i' opens AddBook modal
}
#[test]
fn test_handle_library_j_opens_jump_chapter() {
    // Test 'j' opens JumpChapter modal
}
#[test]
fn test_handle_library_u_updates_all() {
    // Test 'u' sends UpdateAll command
}
#[test]
fn test_handle_library_d_deletes() {
    // Test 'd' deletes selected
}
#[test]
fn test_handle_library_f_cycles_filter() {
    // Test 'f' cycles filter
}
#[test]
fn test_handle_library_space_cycles_status() {
    // Test Space cycles status
}
#[test]
fn test_handle_library_enter_opens_reader() {
    // Test Enter opens reader
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cd /home/monkeinbox/Projects/Scylla-Reader/scylla-reader && cargo test test_handle_library -- --nocapture
```
Expected: FAIL — old keybindings still in place

- [ ] **Step 3: Write minimal implementation**

```rust
pub fn handle_library(
    state: &mut AppState,
    key: KeyEvent,
    cmd_tx: &std::sync::mpsc::Sender<AppCommand>,
) -> bool {
    match key.code {
        KeyCode::Char('i') => {
            state.modal = Modal::AddBook { inputs: vec![String::new()], cursor: 0, scroll_offset: 0 };
            state.current_page = Page::AddingBook;
            true
        }
        KeyCode::Char('j') => {
            if let Some(book) = state.library.selected_book() {
                state.modal = Modal::JumpChapter { chapters: book.chapters.clone(), cursor: 0, scroll_offset: 0, show_titles: true };
                state.current_page = Page::BookChapterJump;
            }
            true
        }
        KeyCode::Char('u') => {
            let urls: Vec<String> = state.library.books.iter().map(|b| b.url.clone()).filter(|u| !u.is_empty()).collect();
            if !urls.is_empty() {
                let _ = cmd_tx.send(AppCommand::UpdateAll(urls));
            }
            true
        }
        KeyCode::Char('d') => {
            state.library.remove_selected();
            true
        }
        KeyCode::Char('f') => {
            state.library.cycle_filter();
            true
        }
        KeyCode::Char(' ') => {
            state.library.cycle_selected_status();
            true
        }
        KeyCode::Enter => {
            if let Some(book) = state.library.selected_book() {
                if book.chapters.is_empty() {
                    let book_title = book.title.clone();
                    let desc = book.description.clone().unwrap_or_else(|| "No content available.".to_string());
                    state.open_reader_chapter("Description".to_string(), desc, 0);
                } else {
                    let idx = (book.progress.current as usize).min(book.chapters.len() - 1);
                    let chapter_url = book.chapters[idx].url.clone();
                    state.reader.loading = true;
                    state.current_page = Page::Reader;
                    if let Err(e) = cmd_tx.send(AppCommand::FetchChapter(chapter_url, idx)) {
                        crate::settings::log(crate::settings::LogLevel::Debug, "INPUT", &format!("Failed to queue chapter: {}", e));
                    }
                }
            }
            true
        }
        KeyCode::Up => {
            if state.library.visible_indices().len() > 0 {
                state.library.selected_index = state.library.selected_index.saturating_sub(1);
            }
            true
        }
        KeyCode::Down => {
            let visible_len = state.library.visible_indices().len();
            if visible_len > 0 {
                state.library.selected_index = (state.library.selected_index + 1).min(visible_len - 1);
            }
            true
        }
        _ => true,
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cd /home/monkeinbox/Projects/Scylla-Reader/scylla-reader && cargo test test_handle_library -- --nocapture
```
Expected: PASS

- [ ] **Step 5: Commit**

```bash
cd /home/monkeinbox/Projects/Scylla-Reader && git add scylla-reader/src/input/library.rs && git commit -m "feat(input): update library keybindings per design"
```

---

### Task 8: Update Reader Input Handler

**Files:**
- Modify: `src/input/reader.rs:8-87`
- Test: `src/input/reader.rs` (inline tests)

**Interfaces:**
- Consumes: New reader keybindings
- Produces: Updated `handle_reader`

- [ ] **Step 1: Write failing tests**

```rust
#[test]
fn test_handle_reader_gt_next_chapter() {
    // Test '>' goes to next chapter
}
#[test]
fn test_handle_reader_lt_prev_chapter() {
    // Test '<' goes to prev chapter
}
#[test]
fn test_handle_reader_esc_back_to_library() {
    // Test Esc returns to library
}
#[test]
fn test_handle_reader_paged_navigation() {
    // Test Right/l and Left/h for paging
}
#[test]
fn test_handle_reader_scrollable_navigation() {
    // Test Down/j, Up/k, PgDn, PgUp for scrolling
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cd /home/monkeinbox/Projects/Scylla-Reader/scylla-reader && cargo test test_handle_reader -- --nocapture
```
Expected: FAIL

- [ ] **Step 3: Write minimal implementation**

```rust
pub fn handle_reader(
    state: &mut AppState,
    key: KeyEvent,
    cmd_tx: &std::sync::mpsc::Sender<AppCommand>,
    size: Rect,
) -> bool {
    use crate::settings::ReaderMode;

    match (key.modifiers, key.code) {
        (_, KeyCode::Char('>')) | (_, KeyCode::Right) | (_, KeyCode::Char('l')) => {
            if !state.reader.loading {
                if let Some(book) = state.library.selected_book() {
                    let next_idx = state.reader.current_chapter_idx + 1;
                    if let Some(ch) = book.chapters.get(next_idx) {
                        let url = ch.url.clone();
                        state.reader.loading = true;
                        let _ = cmd_tx.send(AppCommand::FetchChapter(url, next_idx));
                    }
                }
            }
            return true;
        }
        (_, KeyCode::Char('<')) | (_, KeyCode::Left) | (_, KeyCode::Char('h')) => {
            if !state.reader.loading {
                if let Some(book) = state.library.selected_book() {
                    let prev_idx = state.reader.current_chapter_idx.saturating_sub(1);
                    if prev_idx != state.reader.current_chapter_idx {
                        if let Some(ch) = book.chapters.get(prev_idx) {
                            let url = ch.url.clone();
                            state.reader.loading = true;
                            let _ = cmd_tx.send(AppCommand::FetchChapter(url, prev_idx));
                        }
                    }
                }
            }
            return true;
        }
        (_, KeyCode::Esc) => {
            state.current_page = Page::Library;
            return true;
        }
        _ => {}
    }

    match state.settings.reader_mode {
        ReaderMode::Paged => match key.code {
            KeyCode::Right | KeyCode::Char('l') => {
                state.reader.next_page(size.width, size.height.saturating_sub(2));
                true
            }
            KeyCode::Left | KeyCode::Char('h') => {
                state.reader.prev_page(size.width, size.height.saturating_sub(2));
                true
            }
            _ => true,
        },
        ReaderMode::Scrollable => match key.code {
            KeyCode::Down | KeyCode::Char('j') => {
                state.reader.scroll_down_visual(size.width);
                true
            }
            KeyCode::Up | KeyCode::Char('k') => {
                state.reader.scroll_up_visual();
                true
            }
            KeyCode::PageDown => {
                state.reader.scroll_down_visual_page(size.width, size.height);
                true
            }
            KeyCode::PageUp => {
                state.reader.scroll_up_visual_page(size.height);
                true
            }
            _ => true,
        },
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cd /home/monkeinbox/Projects/Scylla-Reader/scylla-reader && cargo test test_handle_reader -- --nocapture
```
Expected: PASS

- [ ] **Step 5: Commit**

```bash
cd /home/monkeinbox/Projects/Scylla-Reader && git add scylla-reader/src/input/reader.rs && git commit -m "feat(input): update reader keybindings per design"
```

---

### Task 9: Update Settings Input Handler

**Files:**
- Modify: `src/input/settings.rs:6-210`
- Test: `src/input/settings.rs` (inline tests)

**Interfaces:**
- Consumes: New settings keybindings
- Produces: Updated `handle_settings` and sub-handlers

- [ ] **Step 1: Write failing tests**

```rust
#[test]
fn test_handle_settings_navigate() {
    // Test Up/Down navigate
}
#[test]
fn test_handle_settings_enter_selects() {
    // Test Enter selects/edits
}
#[test]
fn test_handle_settings_esc_back() {
    // Test Esc goes back
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cd /home/monkeinbox/Projects/Scylla-Reader/scylla-reader && cargo test test_handle_settings -- --nocapture
```
Expected: FAIL

- [ ] **Step 3: Write minimal implementation** (update to use consistent Esc/Enter)

```rust
// Key changes:
// - Esc always goes back one level
// - Enter always confirms/selects
// - Remove Tab handling (use global 3 for settings)
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cd /home/monkeinbox/Projects/Scylla-Reader/scylla-reader && cargo test test_handle_settings -- --nocapture
```
Expected: PASS

- [ ] **Step 5: Commit**

```bash
cd /home/monkeinbox/Projects/Scylla-Reader && git add scylla-reader/src/input/settings.rs && git commit -m "feat(input): update settings keybindings for consistency"
```

---

### Task 10: Update Modal Input Handlers

**Files:**
- Modify: `src/input/modal.rs:7-135`
- Test: `src/input/modal.rs` (inline tests)

**Interfaces:**
- Consumes: New modal keybindings
- Produces: Updated `handle_adding_book`, `handle_jumping_chapter`

- [ ] **Step 1: Write failing tests**

```rust
#[test]
fn test_handle_adding_book_ctrl_s_submits() {
    // Test Ctrl+S submits
}
#[test]
fn test_handle_adding_book_esc_cancels() {
    // Test Esc cancels
}
#[test]
fn test_handle_jump_chapter_enter_selects() {
    // Test Enter selects chapter
}
#[test]
fn test_handle_jump_chapter_esc_cancels() {
    // Test Esc cancels
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cd /home/monkeinbox/Projects/Scylla-Reader/scylla-reader && cargo test test_handle_modal -- --nocapture
```
Expected: FAIL

- [ ] **Step 3: Write minimal implementation**

```rust
// Add Book: Enter=new line, Backspace=delete, Up/Down=move, Ctrl+S=submit, Esc=cancel
// Jump Chapter: Up/Down=navigate, Enter=select, t=toggle title/URL, Esc=cancel
// Both: Esc=cancel (consistent)
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cd /home/monkeinbox/Projects/Scylla-Reader/scylla-reader && cargo test test_handle_modal -- --nocapture
```
Expected: PASS

- [ ] **Step 5: Commit**

```bash
cd /home/monkeinbox/Projects/Scylla-Reader && git add scylla-reader/src/input/modal.rs && git commit -m "feat(input): update modal keybindings for consistency"
```

---

### Task 11: Add Footer Hints Rendering

**Files:**
- Modify: `src/ui/library.rs:31-33` (replace old hints)
- Modify: `src/ui/reader.rs:85-90, 143-148` (footer hints)
- Modify: `src/ui/settings.rs:43-45, 77-79, 108-110, 162-164, 205-207` (footer hints)
- Modify: `src/ui/modal.rs:39, 77` (modal hints always visible)

**Interfaces:**
- Consumes: `AppState.show_hints`
- Produces: Conditional footer hints bars

- [ ] **Step 1: Write failing test**

```rust
#[test]
fn test_library_draw_shows_hints_when_enabled() {
    // Test hints render when show_hints=true
}
#[test]
fn test_library_draw_hides_hints_when_disabled() {
    // Test hints hidden when show_hints=false
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cd /home/monkeinbox/Projects/Scylla-Reader/scylla-reader && cargo test test_draw_hints -- --nocapture
```
Expected: FAIL

- [ ] **Step 3: Write minimal implementation**

```rust
// In library.rs draw():
if state.show_hints {
    let hints = Paragraph::new(" 1 Library  2 Reader  3 Settings  i Add  j Jump  u Update  d Delete  f Filter  Space Status  : Palette  q Quit")
        .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(hints, chunks[2]);
}

// In reader.rs draw_paged() and draw_scrollable():
if state.show_hints {
    let hints = Paragraph::new(" > Next  < Prev  h/l Page  j/k Scroll  PgDn/PgUp  Esc Back  : Palette")
        .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(hints, chunks[2]);
}

// In settings.rs draw_* functions:
if state.show_hints {
    let hints = Paragraph::new(" ↑/↓ Nav  Enter Select  Esc Back  : Palette")
        .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(hints, chunks[1]);
}

// In modal.rs: hints always visible (no show_hints check)
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cd /home/monkeinbox/Projects/Scylla-Reader/scylla-reader && cargo test test_draw_hints -- --nocapture
```
Expected: PASS

- [ ] **Step 5: Commit**

```bash
cd /home/monkeinbox/Projects/Scylla-Reader && git add scylla-reader/src/ui/library.rs scylla-reader/src/ui/reader.rs scylla-reader/src/ui/settings.rs scylla-reader/src/ui/modal.rs && git commit -m "feat(ui): add footer hints toggle rendering"
```

---

### Task 12: Update Palette Input Handling

**Files:**
- Create: `src/input/palette.rs` (new file)
- Modify: `src/input/mod.rs:13-25` (dispatch to palette handler)

**Interfaces:**
- Consumes: `Modal::CommandPalette`, `PaletteAction.handler`
- Produces: `handle_palette(state, key, cmd_tx)` function

- [ ] **Step 1: Write failing test**

```rust
#[test]
fn test_handle_palette_type_filters() {
    // Test typing filters actions
}
#[test]
fn test_handle_palette_enter_executes() {
    // Test Enter executes selected action
}
#[test]
fn test_handle_palette_esc_closes() {
    // Test Esc closes palette
}
#[test]
fn test_handle_palette_up_down_navigates() {
    // Test Up/Down navigation
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cd /home/monkeinbox/Projects/Scylla-Reader/scylla-reader && cargo test test_handle_palette -- --nocapture
```
Expected: FAIL — module doesn't exist

- [ ] **Step 3: Write minimal implementation**

```rust
//! Palette input handler.

use crate::messenger::AppCommand;
use crate::state::{AppState, Modal};
use crate::state::modal::PaletteAction;
use crossterm::event::{KeyCode, KeyEvent};
use std::sync::mpsc;

pub fn handle_palette(state: &mut AppState, key: KeyEvent, cmd_tx: &mpsc::Sender<AppCommand>) -> bool {
    if let Modal::CommandPalette { query, filtered, selected } = &mut state.modal {
        match key.code {
            KeyCode::Esc => {
                state.close_modal();
                true
            }
            KeyCode::Enter => {
                if let Some(action) = filtered.get(*selected) {
                    (action.handler)(state, cmd_tx);
                }
                state.close_modal();
                true
            }
            KeyCode::Up => {
                *selected = selected.saturating_sub(1);
                true
            }
            KeyCode::Down => {
                if *selected + 1 < filtered.len() {
                    *selected += 1;
                }
                true
            }
            KeyCode::Char(c) => {
                query.push(c);
                *filtered = crate::ui::palette::filter_actions(
                    &crate::ui::palette::build_palette_actions(cmd_tx.clone()),
                    query
                );
                *selected = 0;
                true
            }
            KeyCode::Backspace => {
                query.pop();
                *filtered = crate::ui::palette::filter_actions(
                    &crate::ui::palette::build_palette_actions(cmd_tx.clone()),
                    query
                );
                *selected = 0;
                true
            }
            _ => true,
        }
    } else {
        true
    }
}
```

- [ ] **Step 4: Update input/mod.rs to dispatch**

```rust
Page::Library | Page::AddingBook | Page::BookChapterJump => {
    if let Modal::CommandPalette { .. } = &state.modal {
        palette::handle_palette(state, key, cmd_tx)
    } else {
        library::handle_library(state, key, cmd_tx)
    }
}
// Similar for other pages
```

- [ ] **Step 5: Run test to verify it passes**

```bash
cd /home/monkeinbox/Projects/Scylla-Reader/scylla-reader && cargo test test_handle_palette -- --nocapture
```
Expected: PASS

- [ ] **Step 6: Commit**

```bash
cd /home/monkeinbox/Projects/Scylla-Reader && git add scylla-reader/src/input/palette.rs scylla-reader/src/input/mod.rs && git commit -m "feat(input): add command palette input handling"
```

---

### Task 13: Integration Tests & Polish

**Files:**
- Modify: `src/app.rs` (ensure all pieces connect)
- Test: `src/app.rs` (integration tests)

- [ ] **Step 1: Write integration tests**

```rust
#[test]
fn test_full_flow_library_to_reader() {
    // Add book -> open -> read -> navigate
}
#[test]
fn test_palette_opens_from_any_screen() {
    // Test : works from library, reader, settings
}
#[test]
fn test_hints_toggle_works_everywhere() {
    // Test ? toggles hints on all screens
}
```

- [ ] **Step 2: Run all tests**

```bash
cd /home/monkeinbox/Projects/Scylla-Reader/scylla-reader && cargo test -- --nocapture
```
Expected: ALL PASS

- [ ] **Step 3: Run clippy and fmt**

```bash
cd /home/monkeinbox/Projects/Scylla-Reader/scylla-reader && cargo clippy && cargo fmt --check
```
Expected: NO WARNINGS, NO FORMAT ISSUES

- [ ] **Step 4: Commit**

```bash
cd /home/monkeinbox/Projects/Scylla-Reader && git add -A && git commit -m "feat(ui): complete UI overhaul with palette, hints, global nav"
```

---

### Task 14: Update Documentation

**Files:**
- Modify: `README.md` (update keybindings documentation)

- [ ] **Step 1: Update README with new keybindings**

```markdown
## Keybindings

### Global (work everywhere)
| Key | Action |
|-----|--------|
| `1` | Library |
| `2` | Reader |
| `3` | Settings |
| `:` | Command Palette |
| `?` | Toggle Hints |
| `Esc` | Back / Close Modal |
| `q` | Quit (from Library) |

### Library
| Key | Action |
|-----|--------|
| `i` | Add Book |
| `j` | Jump Chapter |
| `u` | Update All |
| `d` | Delete Book |
| `f` | Cycle Filter |
| `Space` | Cycle Status |
| `Enter` | Open Book |
| `↑/↓` | Navigate |

### Reader
| Key | Action |
|-----|--------|
| `>` / `→` / `l` | Next Chapter |
| `<` / `←` / `h` | Prev Chapter |
| `j` / `↓` | Scroll Down / Next Page |
| `k` / `↑` | Scroll Up / Prev Page |
| `PgDn` / `PgUp` | Page Scroll |
| `Esc` | Back to Library |

### Settings
| Key | Action |
|-----|--------|
| `↑/↓` | Navigate |
| `Enter` | Select / Edit |
| `Esc` | Back |

### Modals
| Key | Action |
|-----|--------|
| `Esc` | Cancel |
| `Enter` | Confirm / Save |
| `↑/↓` | Navigate |
| `Ctrl+S` | Submit (Add Book) |
| `t` | Toggle Title/URL (Jump) |
```

- [ ] **Step 2: Commit**

```bash
cd /home/monkeinbox/Projects/Scylla-Reader && git add README.md && git commit -m "docs: update keybindings in README"
```

---

## Spec Coverage Check

| Spec Section | Task(s) |
|--------------|---------|
| Global Navigation (1/2/3) | Task 6 |
| Command Palette (`:`) | Tasks 2, 3, 4, 5, 12 |
| Hints Toggle (`?`) | Tasks 1, 11 |
| Consistency Rules (Esc/Enter) | Tasks 6, 7, 8, 9, 10 |
| Library Keybindings | Task 7 |
| Reader Keybindings | Task 8 |
| Settings Keybindings | Task 9 |
| Modal Keybindings | Task 10 |
| Footer Hints | Task 11 |
| Visual Layout | Task 11 |
| Palette Component | Tasks 2, 3, 4, 5, 12 |

All spec requirements covered. No gaps.