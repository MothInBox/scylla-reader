# Scylla-Reader Codebase Fixes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Address all 20+ issues from the code review across 7 phases organized by severity.

**Architecture:** Each phase produces independently testable changes. Phases 1-3 fix bugs and performance (user-visible), Phases 4-6 handle cleanup and consistency. The WASM caching in Phase 2 is the largest change, touching the ScraperRegistry's lifecycle.

**Tech Stack:** Rust, ratatui, crossterm, extism, tokio, rusqlite, curl, dirs

---

## Global Constraints

- Rust edition 2024 for main crate, edition 2021 for plugin crates
- All tests must pass: `cargo test` in scylla-reader
- Must build with: `cargo build` in scylla-reader (requires `nix develop` for OpenSSL)
- WASM plugins must still compile: `cargo build --target wasm32-unknown-unknown` in each plugin dir
- Follow existing code conventions (error handling with unwrap_or_else + log, idiomatic Rust)
- No new dependencies unless explicitly required
- Commits should be granular and descriptive

---

### Phase 1: Critical Bug Fixes

#### Task 1: Fix selected_index navigation with active filters

**Files:**
- Modify: `scylla-reader/src/input/library.rs:91-103`

**Interfaces:**
- Consumes: `Library::visible_indices() -> Vec<usize>`
- Produces: Correct navigation clamping against visible count

- [ ] **Step 1: Write failing test**

Add to `scylla-reader/src/library.rs` tests:
```rust
#[test]
fn test_navigation_respects_filter_bounds() {
    let mut lib = Library::new();
    lib.add_book("Book A".into(), "url-a".into(), 10);
    lib.add_book("Book B".into(), "url-b".into(), 10);
    lib.books[1].status = BookStatus::Dropped;

    // Filter to show only Reading books
    lib.filter = LibraryFilter::ByStatus(BookStatus::Reading);

    // Should only have 1 visible item, so selected_index should max at 0
    lib.selected_index = 0;
    lib.select_next(); // should not move to index 1
    assert_eq!(lib.selected_index, 0, "should not navigate past visible items");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd /home/monkeinbox/Projects/Scylla-Reader && cargo test -p scylla-reader library::tests::test_navigation_respects_filter_bounds`
Expected: FAIL

- [ ] **Step 3: Fix the bug**

In `scylla-reader/src/input/library.rs`, change lines 91-103:

```rust
// Before:
KeyCode::Down => {
    if total_books > 0 {
        state.library.selected_index =
            (state.library.selected_index + 1).min(total_books - 1);
    }
    true
}
KeyCode::Up => {
    if total_books > 0 {
        state.library.selected_index = state.library.selected_index.saturating_sub(1);
    }
    true
}

// After:
KeyCode::Down => {
    let visible_len = state.library.visible_indices().len();
    if visible_len > 0 {
        state.library.selected_index =
            (state.library.selected_index + 1).min(visible_len - 1);
    }
    true
}
KeyCode::Up => {
    if state.library.visible_indices().len() > 0 {
        state.library.selected_index = state.library.selected_index.saturating_sub(1);
    }
    true
}
```

Also remove the unused `let total_books = state.library.books.len();` at the top of the function.

- [ ] **Step 4: Run test to verify it passes**

Run: `cd /home/monkeinbox/Projects/Scylla-Reader && cargo test -p scylla-reader library::tests::test_navigation_respects_filter_bounds`
Expected: PASS

- [ ] **Step 5: Run full test suite**

Run: `cd /home/monkeinbox/Projects/Scylla-Reader && cargo test -p scylla-reader`
Expected: All tests pass

- [ ] **Step 6: Commit**

```bash
cd /home/monkeinbox/Projects/Scylla-Reader && git add scylla-reader/src/input/library.rs scylla-reader/src/library.rs && git commit -m "fix: selected_index navigation now clamps against visible_indices() length instead of books.len()"
```

---

#### Task 2: Fix scrollable reader to scroll by visual wrapped lines

**Files:**
- Modify: `scylla-reader/src/state/reader.rs`
- Modify: `scylla-reader/src/ui/reader.rs`

**Interfaces:**
- Consumes: `ReaderState::content: Vec<String>`, `ReaderState::scroll: usize`
- Produces: `ReaderState::visual_scroll: usize`, `ReaderState::total_visual_lines(area_width)`, `ReaderState::visible_wrapped_lines(area_width, area_height)`

- [ ] **Step 1: Add visual_scroll field and visual line methods**

In `scylla-reader/src/state/reader.rs`, add to `ReaderState`:
```rust
pub visual_scroll: usize,
```

Initialize in `new()`:
```rust
visual_scroll: 0,
```

Add methods:
```rust
pub fn total_visual_lines(&self, area_width: u16) -> usize {
    let width = area_width as usize;
    self.content
        .iter()
        .map(|l| Self::wrap_line_count(l, width))
        .sum()
}

pub fn visible_wrapped_lines(&self, area_width: u16, area_height: u16) -> Vec<String> {
    let height = area_height as usize;
    let width = area_width as usize;

    // Build all wrapped lines
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

    // Clamp visual_scroll
    let max_scroll = vlines.len().saturating_sub(height);
    let scroll = std::cmp::min(self.visual_scroll, max_scroll);

    let end = (scroll + height).min(vlines.len());
    vlines[scroll..end].to_vec()
}
```

Update `scroll_down` and `scroll_up` to use visual lines:
```rust
pub fn scroll_down_visual(&mut self, area_width: u16) {
    let total = self.total_visual_lines(area_width);
    if total > 0 {
        self.visual_scroll = (self.visual_scroll + 1).min(total.saturating_sub(1));
    }
}

pub fn scroll_up_visual(&mut self) {
    self.visual_scroll = self.visual_scroll.saturating_sub(1);
}

pub fn scroll_down_visual_page(&mut self, area_width: u16, area_height: u16) {
    let total = self.total_visual_lines(area_width);
    let step = (area_height as usize).saturating_sub(4);
    if total > 0 {
        self.visual_scroll = (self.visual_scroll + step).min(total.saturating_sub(1));
    }
}

pub fn scroll_up_visual_page(&mut self, area_height: u16) {
    let step = (area_height as usize).saturating_sub(4);
    self.visual_scroll = self.visual_scroll.saturating_sub(step);
}
```

Also update `load()` to reset `visual_scroll`:
```rust
pub fn load(&mut self, ...) {
    ...
    self.visual_scroll = 0;
    ...
}
```

- [ ] **Step 2: Update scrollable reader UI**

In `scylla-reader/src/ui/reader.rs`, replace `draw_scrollable`:

```rust
fn draw_scrollable(frame: &mut Frame, area: Rect, state: &AppState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(area);

    let header = Paragraph::new(format!(
        " {} — Ch.{} {}",
        state.reader.book_title,
        state.reader.current_chapter_idx + 1,
        state.reader.chapter_title,
    ))
    .style(Style::default().fg(Color::Yellow));
    frame.render_widget(header, chunks[0]);

    let lines = state
        .reader
        .visible_wrapped_lines(chunks[1].width, chunks[1].height);
    let content = lines.join("\n");
    let block = Block::default().borders(Borders::LEFT);
    let paragraph = Paragraph::new(content)
        .block(block);
    frame.render_widget(paragraph, chunks[1]);

    let total_visual = state.reader.total_visual_lines(chunks[1].width);
    let progress = if total_visual == 0 {
        0
    } else {
        let current = std::cmp::min(state.reader.visual_scroll, total_visual.saturating_sub(1));
        (current * 100) / total_visual
    };

    let prev_hint = if state.reader.current_chapter_idx > 0 {
        "[<] Prev  "
    } else {
        ""
    };
    let next_hint = if state
        .library
        .selected_book()
        .map(|b| state.reader.current_chapter_idx + 1 < b.chapters.len())
        .unwrap_or(false)
    {
        "  [>] Next"
    } else {
        ""
    };
    let footer = Paragraph::new(format!(
        " {}{}%  [j/↓] [k/↑] Scroll  [PgDn/PgUp]{}  [Esc] Library",
        prev_hint, progress, next_hint,
    ))
    .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(footer, chunks[2]);
}
```

- [ ] **Step 3: Update scrollable reader input**

In `scylla-reader/src/input/reader.rs`, update the `ReaderMode::Scrollable` match arm:

```rust
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
```

- [ ] **Step 4: Run tests**

Run: `cd /home/monkeinbox/Projects/Scylla-Reader && cargo test -p scylla-reader`
Expected: All tests pass

- [ ] **Step 5: Commit**

```bash
cd /home/monkeinbox/Projects/Scylla-Reader && git add scylla-reader/src/state/reader.rs scylla-reader/src/ui/reader.rs scylla-reader/src/input/reader.rs && git commit -m "fix: scrollable reader now scrolls by visual wrapped lines instead of raw content lines"
```

---

#### Task 3: Wire rate limit setting to worker

**Files:**
- Modify: `scylla-reader/src/messenger.rs`
- Modify: `scylla-reader/src/worker.rs`
- Modify: `scylla-reader/src/app.rs`
- Modify (minor): `scylla-reader/src/input/settings.rs`

**Interfaces:**
- Consumes: `AppCommand::UpdateAll(Vec<String>)` (unchanged), settings rate_limit_secs
- Produces: `AppCommand::SetRateLimit(u64)`

- [ ] **Step 1: Add SetRateLimit command**

In `scylla-reader/src/messenger.rs`:
```rust
pub enum AppCommand {
    Scrape(String),
    UpdateAll(Vec<String>),
    FetchChapter(String, usize),
    SetRateLimit(u64),
}
```

- [ ] **Step 2: Add rate_limit field to Worker**

In `scylla-reader/src/worker.rs`:
```rust
pub struct Worker {
    cmd_rx: mpsc::Receiver<AppCommand>,
    event_tx: mpsc::Sender<AppEvent>,
    registry: ScraperRegistry,
    rate_limit_secs: u64,
}

impl Worker {
    pub fn new(
        cmd_rx: mpsc::Receiver<AppCommand>,
        event_tx: mpsc::Sender<AppEvent>,
        registry: ScraperRegistry,
    ) -> Self {
        Self { cmd_rx, event_tx, registry, rate_limit_secs: 2 }
    }
    ...
```

- [ ] **Step 3: Handle SetRateLimit and use rate limit in UpdateAll**

In the `run()` match:
```rust
AppCommand::SetRateLimit(secs) => {
    self.rate_limit_secs = secs;
}
```

Change `UpdateAll` sleep:
```rust
// Before:
std::thread::sleep(std::time::Duration::from_secs(2));
// After:
std::thread::sleep(std::time::Duration::from_secs(self.rate_limit_secs));
```

- [ ] **Step 4: Send rate limit from settings when it changes**

In `scylla-reader/src/input/settings.rs`, in `handle_settings_main`, after the rate limit Enter handler writes to the buffer, we need to send it to the worker when saved. The settings input handler doesn't currently have access to `cmd_tx`. Add `cmd_tx` parameter:

```rust
pub fn handle_settings(
    state: &mut AppState,
    key: KeyEvent,
    cmd_tx: &std::sync::mpsc::Sender<AppCommand>,
) -> bool {
    // pass cmd_tx through to sub-handlers
}
```

Pass `cmd_tx` from `input::handle_input` and through to sub-handlers that need it (rate limit save).

In `scylla-reader/src/input/settings.rs`, when rate limit editing finishes (Enter key while editing), send:
```rust
if let Ok(rate) = state.settings.edit_buffer.parse::<u64>() {
    state.settings.rate_limit_secs = rate;
    let _ = cmd_tx.send(AppCommand::SetRateLimit(rate));
}
```

- [ ] **Step 5: Wire cmd_tx through input dispatch**

In `scylla-reader/src/input/mod.rs`:
```rust
Page::Settings => settings::handle_settings(state, key, cmd_tx),
```

- [ ] **Step 6: Run tests**

Run: `cd /home/monkeinbox/Projects/Scylla-Reader && cargo test -p scylla-reader`
Expected: All tests pass

- [ ] **Step 7: Commit**

```bash
cd /home/monkeinbox/Projects/Scylla-Reader && git add scylla-reader/src/messenger.rs scylla-reader/src/worker.rs scylla-reader/src/app.rs scylla-reader/src/input/mod.rs scylla-reader/src/input/settings.rs && git commit -m "fix: wire rate limit setting to worker through new AppCommand::SetRateLimit"
```

---

### Phase 2: Performance — WASM Plugin Caching

#### Task 4: Cache WASM Plugin instances in ScraperRegistry

**Files:**
- Modify: `scylla-reader/src/scrapers/services.rs`

**Interfaces:**
- Consumes: `ScraperRegistry` methods `scrape_url`, `scrape_chapter`
- Produces: Cached `Plugin` instances per domain (lazily initialized)

- [ ] **Step 1: Add plugin cache to ScraperRegistry**

Change `ScraperRegistry` to hold cached plugins:
```rust
use std::cell::RefCell;

pub struct ScraperRegistry {
    plugins: Vec<(String, std::path::PathBuf)>,
    plugin_cache: RefCell<Vec<(String, extism::Plugin)>>,
}
```

Initialize in `new()`:
```rust
Self { plugins, plugin_cache: RefCell::new(Vec::new()) }
```

- [ ] **Step 2: Add cache lookup/init method**

```rust
impl ScraperRegistry {
    fn get_or_init_plugin(&self, domain: &str, wasm_path: &std::path::PathBuf) -> Result<std::cell::RefMut<extism::Plugin>, Box<dyn std::error::Error + Send + Sync>> {
        // Check cache first
        let idx = {
            let cache = self.plugin_cache.borrow();
            cache.iter().position(|(d, _)| d == domain)
        };

        if let Some(idx) = idx {
            // Return cached plugin via RefMut mapping — but RefCell doesn't support that directly.
            // Instead, we need a different approach. Let's use a Vec and index-based access.
        }

        // Not found — create new plugin
        let curl_fetch_fn = extism::Function::new(
            "curl_fetch",
            [extism::ValType::I64],
            [extism::ValType::I64],
            extism::UserData::<()>::default(),
            host_curl_fetch,
        );
        let wasm = extism::Wasm::file(wasm_path);
        let manifest = extism::Manifest::new([wasm]).with_allowed_host("*");
        let plugin = extism::Plugin::new(&manifest, [curl_fetch_fn], true)?;

        // Store in cache — but Plugin requires &mut self for call, so we need interior mutability.
        // Simplest: use RefCell<Vec<Plugin>> and borrow_mut to get a mutable reference by index.

        let idx = {
            let mut cache = self.plugin_cache.borrow_mut();
            cache.push((domain.to_string(), plugin));
            cache.len() - 1
        };

        // Return — but we can't return a RefMut to a specific element. Instead, we need
        // a different caching strategy. See Step 3.
    }
}
```

**Simpler approach:** Use `RefCell<HashMap<String, extism::Plugin>>` and add a method that returns `Plugin` by cloning. Actually, `Plugin` might not be `Clone`. Let's use a different approach.

**Alternative: Use a Vec-based index approach.** On every call, look up the cache, borrow_mut the entire cache, find the right plugin, call it, and return.

```rust
fn call_cached_plugin(
    &self,
    domain: &str,
    wasm_path: &std::path::PathBuf,
    function: &str,
    input: &[u8],
) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    let mut cache = self.plugin_cache.borrow_mut();
    let idx = cache.iter().position(|(d, _)| d == domain);

    let plugin = if let Some(idx) = idx {
        &mut cache[idx].1
    } else {
        // Create new plugin
        let curl_fetch_fn = extism::Function::new(
            "curl_fetch",
            [extism::ValType::I64],
            [extism::ValType::I64],
            extism::UserData::<()>::default(),
            host_curl_fetch,
        );
        let wasm = extism::Wasm::file(wasm_path);
        let manifest = extism::Manifest::new([wasm]).with_allowed_host("*");
        let plugin = extism::Plugin::new(&manifest, [curl_fetch_fn], true)?;
        cache.push((domain.to_string(), plugin));
        &mut cache.last_mut().unwrap().1
    };

    let result = plugin.call::<&[u8], &[u8]>(function, input)?;
    Ok(result.to_vec())
}
```

- [ ] **Step 3: Replace call_plugin usage**

In `scrape_url` and `scrape_chapter`, replace:
```rust
let output_bytes = call_plugin(wasm_path, "scrape_book", &input_json)?;
```
With:
```rust
let output_bytes = self.call_cached_plugin(domain, wasm_path, "scrape_book", &input_json)?;
```

And similarly for `scrape_chapter`.

- [ ] **Step 4: Keep the free function for discover_schema**

The `call_plugin` free function can still be used by `discover_schema` which runs before caching is set up (during `ScraperRegistry::new`).

- [ ] **Step 5: Run tests**

Run: `cd /home/monkeinbox/Projects/Scylla-Reader && cargo test -p scylla-reader`
Expected: All tests pass

- [ ] **Step 6: Commit**

```bash
cd /home/monkeinbox/Projects/Scylla-Reader && git add scylla-reader/src/scrapers/services.rs && git commit -m "perf: cache WASM Plugin instances in ScraperRegistry to avoid re-compilation per call"
```

---

### Phase 3: Performance — Worker & Cover Improvements

#### Task 5: Fix UpdateAll blocking the worker

**Files:**
- Modify: `scylla-reader/src/worker.rs`

**Interfaces:**
- Consumes: `ScraperRegistry`, `tokio::runtime::Runtime`
- Produces: Async scraping tasks spawned concurrently

- [ ] **Step 1: Replace blocking sleep with async tasks**

In `scylla-reader/src/worker.rs`, change `UpdateAll` handler:

```rust
AppCommand::UpdateAll(urls) => {
    let registry_ref = &self.registry;
    let event_tx = self.event_tx.clone();
    let rate_limit = self.rate_limit_secs;

    for url in urls {
        let url = clean_url(&url);
        let event_tx = event_tx.clone();

        runtime.spawn(async move {
            // Small delay between each update
            tokio::time::sleep(std::time::Duration::from_secs(rate_limit)).await;

            // But we need sync access to registry... registry is not Send.
            // Alternative: use runtime.block_on for each scrape, but spawn the sleep.
        });
    }
}
```

**Issue:** `ScraperRegistry` is not `Send + Sync` (it has `RefCell` now). We can't move it into async tasks.

**Better approach:** Keep scraping synchronous but use `runtime.spawn_blocking` for the sleep or use a timer-based approach:

```rust
AppCommand::UpdateAll(urls) => {
    let rate_limit = self.rate_limit_secs;
    for (i, url) in urls.iter().enumerate() {
        if i > 0 {
            std::thread::sleep(std::time::Duration::from_secs(rate_limit));
        }
        Self::scrape_and_send(&runtime, &self.registry, &self.event_tx, &clean_url(url));
    }
}
```

This is already the current approach but with a fix: only sleep between items (not before the first). More importantly, it does sequential but non-blocking processing by using `runtime.block_on` for each scrape. The key improvement is: if a `FetchChapter` arrives during `UpdateAll`, it won't be processed until `UpdateAll` finishes because the worker runs a single-threaded command loop.

**Real fix:** Process `UpdateAll` asynchronously by spawning each scrape in a separate OS thread and using `tokio::sync::oneshot` or a join set, while still polling the `cmd_rx` for new commands.

Simplest real fix using `tokio::spawn_blocking`:
```rust
AppCommand::UpdateAll(urls) => {
    for url in urls {
        let url = clean_url(&url);
        let registry = self.registry.clone(); // Need ScraperRegistry to be Clone
        // ...
    }
}
```

If `ScraperRegistry` isn't clone (it won't be with internal cache), we can use a different approach: process updates in the background but poll `cmd_rx` with a timeout:

```rust
AppCommand::UpdateAll(urls) => {
    let mut iter = urls.into_iter();
    // Process first immediately
    if let Some(url) = iter.next() {
        Self::scrape_and_send(&runtime, &self.registry, &self.event_tx, &clean_url(&url));
    }
    // Process rest with rate limit, but don't block FetchChapter indefinitely
    for url in iter {
        // Use non-blocking sleep approach: check for new commands while sleeping
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(self.rate_limit_secs);
        while std::time::Instant::now() < deadline {
            // Check if any new commands arrived
            if let Ok(cmd) = self.cmd_rx.recv_timeout(std::time::Duration::from_millis(100)) {
                match cmd {
                    AppCommand::FetchChapter(url, idx) => {
                        // Process the chapter fetch immediately
                        let url = clean_url(&url);
                        match runtime.block_on(self.registry.scrape_chapter(&url)) {
                            Ok((title, content)) => {
                                let _ = self.event_tx.send(AppEvent::ChapterFetched(ChapterContent {
                                    chapter_idx: idx, title, content,
                                }));
                            }
                            Err(e) => crate::settings::log(...),
                        }
                    }
                    AppCommand::SetRateLimit(secs) => self.rate_limit_secs = secs,
                    AppCommand::Scrape(url) => {
                        Self::scrape_and_send(&runtime, &self.registry, &self.event_tx, &clean_url(&url));
                    }
                    AppCommand::UpdateAll(_) => {} // ignore nested UpdateAll
                }
            }
        }
        Self::scrape_and_send(&runtime, &self.registry, &self.event_tx, &clean_url(&url));
    }
}
```

- [ ] **Step 2: Remove unused `total_books` variable**

Already handled in Task 1.

- [ ] **Step 3: Run tests**

Run: `cd /home/monkeinbox/Projects/Scylla-Reader && cargo test -p scylla-reader`
Expected: All tests pass

- [ ] **Step 4: Commit**

```bash
cd /home/monkeinbox/Projects/Scylla-Reader && git add scylla-reader/src/worker.rs && git commit -m "fix: UpdateAll now polls cmd_rx during rate-limit delays instead of blocking entirely"
```

---

#### Task 6: Move cover loading to worker thread

**Files:**
- Modify: `scylla-reader/src/messenger.rs`
- Modify: `scylla-reader/src/worker.rs`
- Modify: `scylla-reader/src/app.rs`

**Interfaces:**
- Consumes: Cover URL from selected book, `reqwest::blocking` (workers), `image` crate, `ratatui_image::picker::Picker`
- Produces: `AppCommand::FetchCover(String)`, `AppEvent::CoverFetched(String, StatefulProtocol)`

**Note:** This task requires `ratatui-image` and `image` on the worker thread. Since cover loading uses `reqwest::blocking` + `image` + `Picker` (all sync), it needs to run as `spawn_blocking` or in a dedicated OS thread.

- [ ] **Step 1: Add command and event types**

In `scylla-reader/src/messenger.rs`:
```rust
pub enum AppCommand {
    Scrape(String),
    UpdateAll(Vec<String>),
    FetchChapter(String, usize),
    SetRateLimit(u64),
    FetchCover(String),
}

pub enum AppEvent {
    BookScraped(Book),
    ChapterFetched(ChapterContent),
    CoverFetched(String, ratatui_image::protocol::StatefulProtocol),
}
```

- [ ] **Step 2: Handle FetchCover in worker**

In `scylla-reader/src/worker.rs`, add to the main `match`:
```rust
AppCommand::FetchCover(url) => {
    let event_tx = self.event_tx.clone();
    std::thread::spawn(move || {
        let mut picker = ratatui_image::picker::Picker::from_fontsize((8, 12));
        match reqwest::blocking::get(&url) {
            Ok(resp) => match resp.bytes() {
                Ok(bytes) => match image::load_from_memory(&bytes) {
                    Ok(img) => {
                        let protocol = picker.new_resize_protocol(img);
                        let _ = event_tx.send(AppEvent::CoverFetched(url, protocol));
                    }
                    Err(e) => { /* log */ }
                },
                Err(e) => { /* log */ }
            },
            Err(e) => { /* log */ }
        }
    });
}
```

- [ ] **Step 3: Remove cover loading from main thread**

In `scylla-reader/src/app.rs`:
- Remove `cover_tx`, `cover_rx`, `picker_font_size`, `picker_protocol_type` fields
- Remove `Picker` creation from `App::new()`
- Replace `update_covers` to send `FetchCover` command instead of spawning threads
- Handle `CoverFetched` event in `drain_events` (replace the `cover_rx.try_recv` block)

```rust
// In App struct, remove:
// cover_tx: mpsc::Sender<(String, StatefulProtocol)>,
// cover_rx: mpsc::Receiver<(String, StatefulProtocol)>,
// picker_font_size: (u16, u16),
// picker_protocol_type: ProtocolType,

// In App::new(), remove:
// let (cover_tx, cover_rx) = mpsc::channel::<(String, StatefulProtocol)>();
// and Picker creation / font_size / protocol_type

// New update_covers:
fn update_covers(&mut self) {
    let current_cover_url = self.state
        .library
        .selected_book()
        .and_then(|b| b.cover_url.clone());

    if current_cover_url != self.last_cover_url {
        self.last_cover_url = current_cover_url.clone();
        self.state.library.cached_protocol = None;

        if let Some(url) = current_cover_url {
            let _ = self.cmd_tx.send(AppCommand::FetchCover(url));
        }
    }
}

// In drain_events, add:
AppEvent::CoverFetched(url, protocol) => {
    let current_url = self.state
        .library
        .selected_book()
        .and_then(|b| b.cover_url.as_deref().map(str::to_owned));
    if current_url.as_deref() == Some(&url) {
        self.state.library.cached_protocol = Some(protocol);
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cd /home/monkeinbox/Projects/Scylla-Reader && cargo test -p scylla-reader`
Expected: All tests pass (build may fail if import issues)

- [ ] **Step 5: Commit**

```bash
cd /home/monkeinbox/Projects/Scylla-Reader && git add scylla-reader/src/messenger.rs scylla-reader/src/worker.rs scylla-reader/src/app.rs && git commit -m "perf: move cover loading from unbounded main-thread spawns to worker thread"
```

---

### Phase 4: Edge Case & Robustness Fixes

#### Task 7: Fix clean_url parenthesization

**Files:**
- Modify: `scylla-reader/src/worker.rs:70-75`

- [ ] **Step 1: Write failing test**

```rust
#[test]
fn test_clean_url_trailing_parens_text() {
    let input = "[link](https://url.com) and (stuff)";
    assert_eq!(clean_url(input), "https://url.com");
}
```

- [ ] **Step 2: Fix clean_url**

Replace with depth-tracking:
```rust
fn clean_url(url: &str) -> String {
    if let (Some(open), _) = (url.find("]("), url.rfind(')')) {
        let after = open + 2;
        let mut depth = 0i32;
        for (i, ch) in url[after..].char_indices() {
            match ch {
                '(' => depth += 1,
                ')' => {
                    if depth == 0 {
                        return url[after..after + i].trim().to_string();
                    }
                    depth -= 1;
                }
                _ => {}
            }
        }
    }
    url.trim().to_string()
}
```

- [ ] **Step 3: Run tests**

Run: `cd /home/monkeinbox/Projects/Scylla-Reader && cargo test -p scylla-reader worker::tests`
Expected: All worker tests pass including new one

- [ ] **Step 4: Commit**

```bash
cd /home/monkeinbox/Projects/Scylla-Reader && git add scylla-reader/src/worker.rs && git commit -m "fix: clean_url uses depth-tracking to find matching closing paren"
```

---

#### Task 8: Fix foreign keys pragma + dead code cleanup

**Files:**
- Modify: `scylla-reader/src/db.rs`
- Modify: `scylla-reader/src/library.rs`
- Modify: `scylla-reader/Cargo.toml`
- Modify: `scylla-reader/src/app.rs`

- [ ] **Step 1: Move pragma to connection open**

In `scylla-reader/src/db.rs`:
```rust
pub fn open() -> Result<Self> {
    let path = data_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let conn = Connection::open(&path)?;
    conn.execute_batch("PRAGMA foreign_keys = ON;")?;  // ADD HERE
    let db = Self { conn };
    db.migrate()?;
    Ok(db)
}

pub fn open_conn(conn: Connection) -> Result<Db> {
    conn.execute_batch("PRAGMA foreign_keys = ON;")?;  // ADD HERE
    let db = Db { conn };
    db.migrate()?;
    Ok(db)
}

pub fn open_path(path: &std::path::Path) -> Result<Db> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let conn = Connection::open(path)?;
    conn.execute_batch("PRAGMA foreign_keys = ON;")?;  // ADD HERE
    let db = Db { conn };
    db.migrate()?;
    Ok(db)
}
```

Remove from `migrate()`:
```rust
fn migrate(&self) -> Result<()> {
    self.conn.execute_batch("
        CREATE TABLE IF NOT EXISTS books (
            ...
        );
        ...
        -- REMOVED: PRAGMA foreign_keys = ON;
    ")
}
```

- [ ] **Step 2: Remove once_cell from Cargo.toml**

Remove: `once_cell = "1.19"` (line 14)

- [ ] **Step 3: Remove set_chapter from library.rs**

Remove the `set_chapter` method and its associated tests.

- [ ] **Step 4: Remove cached_cover and cached_cover_url fields**

In `scylla-reader/src/library.rs`:
```rust
// Remove:
// pub cached_cover: Option<DynamicImage>,
// pub cached_cover_url: Option<String>,
```

Remove initializers in `new()`.

In `scylla-reader/src/app.rs`, remove:
```rust
// self.state.library.cached_cover = None;
// self.state.library.cached_cover_url = None;
```
(lines 136-137, 176-177)

- [ ] **Step 5: Run tests**

Run: `cd /home/monkeinbox/Projects/Scylla-Reader && cargo test -p scylla-reader`
Expected: All tests pass

- [ ] **Step 6: Commit**

```bash
cd /home/monkeinbox/Projects/Scylla-Reader && git add scylla-reader/src/db.rs scylla-reader/src/library.rs scylla-reader/src/app.rs scylla-reader/Cargo.toml && git commit -m "cleanup: move PRAGMA foreign_keys to connection open, remove dead code (set_chapter, cached_cover fields, once_cell dep)"
```

---

#### Task 9: Fix config_dir inconsistency + hardcoded Referer

**Files:**
- Modify: `scylla-reader/src/plugin_config.rs`
- Modify: `scylla-reader/src/scrapers/services.rs`

- [ ] **Step 1: Fix config_dir() to use dirs crate**

```rust
pub fn config_dir() -> PathBuf {
    dirs::config_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("scylla-reader")
}
```

- [ ] **Step 2: Fix hardcoded Referer**

In `services.rs`, remove the hardcoded Referer header or make it dynamic:

```rust
// Remove these lines from fetch_with_curl:
// list.append("Referer: https://www.scribblehub.com/")
//     .map_err(|e| e.to_string())?;
```

Or make it dynamic based on URL domain:
```rust
let domain = url.split('/').nth(2).unwrap_or("");
let referer = format!("https://{}/", domain);
list.append(&format!("Referer: {}", referer))
    .map_err(|e| e.to_string())?;
```

- [ ] **Step 3: Run tests**

Run: `cd /home/monkeinbox/Projects/Scylla-Reader && cargo test -p scylla-reader`
Expected: All tests pass

- [ ] **Step 4: Commit**

```bash
cd /home/monkeinbox/Projects/Scylla-Reader && git add scylla-reader/src/plugin_config.rs scylla-reader/src/scrapers/services.rs && git commit -m "fix: use dirs::config_local_dir() instead of HOME, dynamic Referer header per domain"
```

---

### Phase 5: UI & Display Polish

#### Task 10: Fix scrollable percentage + log file path

**Files:**
- Modify: `scylla-reader/src/ui/reader.rs`
- Modify: `scylla-reader/src/settings/mod.rs`

- [ ] **Step 1: Fix scrollable percentage**

Already fixed in Task 2 (uses visual progress). Verify.

- [ ] **Step 2: Fix log file path**

In `scylla-reader/src/settings/mod.rs`:
```rust
pub const LOG_FILE: &str = "/tmp/scylla-reader.log";  // Remove this

// Replace with:
pub fn log_file() -> std::path::PathBuf {
    let base = dirs::state_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"));
    base.join("scylla-reader").join("scylla-reader.log")
}
```

Update `log()` function to use `log_file()`.
Update `reload_log()` to use `log_file()`.
Update `handle_debug_log()` in `input/settings.rs` to use `log_file()`.

Add directory creation:
```rust
pub fn log(level: LogLevel, module: &str, msg: &str) {
    let enabled = match level {
        LogLevel::Error => true,
        LogLevel::Debug => DEBUG_ENABLED.load(Ordering::Relaxed),
    };
    if !enabled {
        return;
    }
    let ts = timestamp();
    let path = log_file();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(file, "[{}] [{}] [{}] {}", ts, level, module, msg);
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cd /home/monkeinbox/Projects/Scylla-Reader && cargo test -p scylla-reader`
Expected: All tests pass

- [ ] **Step 4: Commit**

```bash
cd /home/monkeinbox/Projects/Scylla-Reader && git add scylla-reader/src/settings/mod.rs scylla-reader/src/input/settings.rs && git commit -m "fix: move log file from /tmp to ~/.local/state/scylla-reader/"
```

---

#### Task 11: Fix Picker reconstruction

**Files:**
- Modify: `scylla-reader/src/app.rs`

**Note:** If Task 6 (cover loading moved to worker) was completed, this is already resolved. Otherwise, this task stores a `Picker` instance directly.

- [ ] **Step 1: Store Picker in App instead of reconstruction params**

```rust
pub struct App {
    terminal: Terminal<CrosstermBackend<std::io::Stdout>>,
    state: AppState,
    cmd_tx: mpsc::Sender<AppCommand>,
    event_rx: mpsc::Receiver<AppEvent>,
    cover_tx: mpsc::Sender<(String, StatefulProtocol)>,
    cover_rx: mpsc::Receiver<(String, StatefulProtocol)>,
    picker: ratatui_image::picker::Picker,  // ADD
    last_cover_url: Option<String>,
}
```

Use `self.picker` in `update_covers` instead of creating a new one.

- [ ] **Step 2: Run tests**

Run: `cd /home/monkeinbox/Projects/Scylla-Reader && cargo test -p scylla-reader`
Expected: Build succeeds, tests pass

- [ ] **Step 3: Commit**

```bash
cd /home/monkeinbox/Projects/Scylla-Reader && git add scylla-reader/src/app.rs && git commit -m "refactor: store Picker instance directly instead of reconstructing from saved params"
```

---

### Phase 6: Plugin HTML Entity Decoding

#### Task 12: Fix incomplete HTML entity decoding in plugins

**Files:**
- Modify: `~/Projects/plugin-royalroad/src/lib.rs`
- Modify: `~/Projects/plugin-scribblehub/src/lib.rs`

- [ ] **Step 1: Extend decode_entities in both plugins**

Add numeric entity decoding to both plugins' `decode_entities`:

```rust
fn decode_entities(s: &str) -> String {
    let s = s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ")
        .replace("&#x27;", "'")
        .replace("&#x2F;", "/")
        .replace("\u{00a0}", " ");
    
    // Decode numeric entities: &#NNN; and &#xHH;
    let mut result = String::new();
    let mut chars = s.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '&' {
            let mut entity = String::new();
            while let Some(&next) = chars.peek() {
                if next == ';' {
                    chars.next();
                    break;
                }
                entity.push(next);
                chars.next();
            }
            if entity.starts_with("#x") || entity.starts_with("#X") {
                if let Ok(code) = u32::from_str_radix(&entity[2..], 16) {
                    if let Some(c) = char::from_u32(code) {
                        result.push(c);
                        continue;
                    }
                }
            } else if entity.starts_with('#') {
                if let Ok(code) = entity[1..].parse::<u32>() {
                    if let Some(c) = char::from_u32(code) {
                        result.push(c);
                        continue;
                    }
                }
            }
            // If we couldn't decode it, preserve the original
            result.push('&');
            result.push_str(&entity);
            result.push(';');
        } else {
            result.push(ch);
        }
    }
    result
}
```

- [ ] **Step 2: Build both plugins**

Run: `cd ~/Projects/plugin-royalroad && cargo build --target wasm32-unknown-unknown`
Run: `cd ~/Projects/plugin-scribblehub && cargo build --target wasm32-unknown-unknown`
Expected: Both build successfully

- [ ] **Step 3: Commit**

```bash
cd ~/Projects/plugin-royalroad && git add src/lib.rs && git commit -m "fix: decode numeric HTML entities (&#NNN; and &#xHH;) in addition to named entities"
cd ~/Projects/plugin-scribblehub && git add src/lib.rs && git commit -m "fix: decode numeric HTML entities (&#NNN; and &#xHH;) in addition to named entities"
```
