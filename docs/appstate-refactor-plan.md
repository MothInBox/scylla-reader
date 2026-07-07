# AppState Refactor Plan

## Problem

`AppState` is a God object — 9 public fields, every handler gets `&mut AppState` and can touch anything. No compile-time access control, hard to reason about data flow, borrow checker fights.

## Plan: 3 Phases

### Phase 1 — Group fields into sub-structs (mechanical)

Break `AppState` into:

```rust
pub struct AppState {
    pub ui: UiState,           // page, modal, show_hints
    pub library: LibraryState, // library, settings, settings_ui
    pub reader: ReaderState,   // unchanged
    pub jobs: JobsState,       // unchanged
    pub db: Db,                // unchanged
}
```

Update ~80 call sites (`state.current_page` → `state.ui.page`, etc.). Pure renaming, no behavior change. Low risk.

### Phase 2 — Focused dispatchers (medium)

Change function signatures so input handlers, UI renderers, and event handlers receive only the sub-structs they need:

```
handle_library_input(library: &mut LibraryState, ...)
handle_reader_input(reader: &mut ReaderState, ...)
handle_settings_input(library: &mut LibraryState, ...)
```

The dispatcher in `input/mod.rs` and `ui/mod.rs` destructures `AppState` and passes the right pieces.

### Phase 3 — Encapsulation

Make `AppState` fields `pub(super)`, add focused accessors. At this point `AppState` is just an ownership container.

## Order

| Step | What | Files |
|------|------|-------|
| 1 | Create `UiState`, `LibraryState` | `state/mod.rs`, `state/ui.rs`, `state/library.rs` |
| 2 | Rename call sites | ~20 files across input/, ui/, state/, event.rs, key_handler.rs |
| 3 | Narrow input handler signatures | `input/*.rs` |
| 4 | Narrow UI renderer signatures | `ui/*.rs` |
| 5 | Narrow event handler signatures | `event.rs` |
| 6 | Mark fields `pub(super)` | `state/mod.rs` |

Steps 3–5 can be parallelized via sub-agents.

## Verification

- Each intermediate commit must pass `cargo test` (currently 282 tests)
- No behavior changes
