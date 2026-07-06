# Codebase Fixes and Architecture Refactor

> **For agentic workers:** Use superpowers:subagent-driven-development to implement this plan task-by-task.

**Goal:** Fix all identified bugs and architectural issues across the Scylla-Reader codebase.

**Architecture:** Rust TUI (ratatui + crossterm), single-threaded main loop + background worker thread (Tokio + Extism WASM). Unified `AppState` struct.

## Execution Phases

To avoid file conflicts, execute in three phases:

- **Phase 1 (parallel):** Bug fixes + state architecture + modal split (no overlapping files)
- **Phase 2:** Keybinding registry + app.rs extraction (touches most files, runs after Phase 1)
- **Phase 3:** Test coverage (can overlap with Phase 2)

---

## Phase 1 Tasks

### 1.1: Fix db.rs Duplicate Tests + input/reader.rs Overflow

**Files:**
- `scylla-reader/src/db.rs` — remove duplicate `#[cfg(test)] mod tests { }` at lines 349-622
- `scylla-reader/src/input/reader.rs` — change `current_chapter_idx + 1` to `.saturating_add(1)`
- `scylla-reader/src/state/reader.rs` — add `total_pages: usize` field; remove terminal dim params from `next_page`/`prev_page`
- `scylla-reader/src/ui/reader.rs` — set `reader.total_pages` before rendering

### 1.2: Fix Worker + Messenger Bugs

**Files:**
- `scylla-reader/src/messenger.rs` — add `ChapterFetchFailed` to `AppEvent`
- `scylla-reader/src/worker.rs` — send error events on chapter failure; use `runtime.spawn_blocking` instead of `thread::spawn` for covers; queue `FetchCover` commands during `UpdateAll` batch
- `scylla-reader/src/app.rs` — handle `ChapterFetchFailed` in `drain_events`

### 1.3: State Architecture Refactor

**Files:**
- Create `scylla-reader/src/state/palette_action.rs` — extract `PaletteAction` from `state/modal.rs`
- Create `scylla-reader/src/state/settings_ui.rs` — extract UI-state fields from `Settings`
- Modify `scylla-reader/src/state/mod.rs` — add new modules + `settings_ui` to `AppState`
- Modify `scylla-reader/src/state/modal.rs` — remove `PaletteAction`
- Modify `scylla-reader/src/settings/mod.rs` — remove transient UI fields from `Settings`
- Modify `scylla-reader/src/input/settings.rs` — update field paths
- Modify `scylla-reader/src/ui/settings.rs` — update field paths

### 1.4: Split input/modal.rs + Add Confirmation Prompt

**Files:**
- Create directory `scylla-reader/src/input/modal/` with `mod.rs`, `add_book.rs`, `jump_chapter.rs`, `session_picker.rs`
- Delete `scylla-reader/src/input/modal.rs`
- Modify `scylla-reader/src/input/modal/session_picker.rs` — add `pending_delete_url` confirmation
- Modify `scylla-reader/src/state/modal.rs` — add `pending_delete_url` field to `SessionPicker`
- Modify `scylla-reader/src/ui/modal.rs` — show confirmation in hints

---

## Phase 2

### 2.1: Keybinding Registry

**Files:**
- Create `scylla-reader/src/input/keybinds.rs` — all keybinding constants
- Modify `scylla-reader/src/input/mod.rs` — add module
- Modify ALL `scylla-reader/src/input/*.rs` files — use keybinding constants
- Modify `scylla-reader/src/ui/palette.rs` — update key strings
- Modify `scylla-reader/src/app.rs` — use keybinding constants

### 2.2: Extract app.rs

**Files:**
- Create `scylla-reader/src/event.rs` — `drain_events` + `update_covers`
- Create `scylla-reader/src/key_handler.rs` — `handle_key`
- Modify `scylla-reader/src/app.rs` — simplify to core orchestration
- Modify `scylla-reader/src/main.rs` — add new modules if needed

---

## Phase 3

### 3.1: Test Coverage

**Files:**
- `scylla-reader/src/scrapers/services.rs` — add test module
- `scylla-reader/src/state/modal.rs` — add structural tests
- `scylla-reader/src/messenger.rs` — add enum tests
- `scylla-reader/src/input/mod.rs` — add dispatch tests
- `scylla-reader/src/ui/mod.rs` — add dispatch tests
- Various `ui/*.rs` files — enhance rendering assertions
