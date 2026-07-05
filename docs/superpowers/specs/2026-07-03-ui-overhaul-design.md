# UI Overhaul Design

**Date:** 2026-07-03  
**Project:** Scylla Reader  
**Status:** Approved

---

## 1. Goals

- Reduce keybinding cognitive load
- Improve discoverability (on-screen hints + command palette)
- Establish consistent patterns (Esc=back, Enter=confirm/open)
- Add global navigation (1/2/3 for screens)
- Preserve existing functionality and muscle memory for core actions

---

## 2. Navigation Paradigm

**Hybrid: Global Numbers + Contextual Letters + Command Palette + Hints Toggle**

| Key | Type | Action |
|-----|------|--------|
| `1` | Global | Library screen |
| `2` | Global | Reader screen |
| `3` | Global | Settings screen |
| `i` | Contextual | Add Book modal |
| `j` | Contextual | Jump Chapter modal |
| `u` | Contextual | Update All Books |
| `d` | Contextual | Delete selected book |
| `f` | Contextual | Cycle Filter |
| `Space` | Contextual | Cycle Status |
| `>` / `Right` / `l` | Contextual | Next chapter (Reader) |
| `<` / `Left` / `h` | Contextual | Prev chapter (Reader) |
| `j` / `↓` | Contextual | Scroll down / Next page |
| `k` / `↑` | Contextual | Scroll up / Prev page |
| `Esc` | Universal | Back / Close modal / Quit (from Library) |
| `:` | Universal | Open Command Palette |
| `?` | Universal | Toggle footer hints |
| `q` | Contextual | Quit (from Library) |

---

## 3. Command Palette (`:`)

**Modal overlay** (60% width, 40% height, centered)

**Structure:** Hybrid global + context sections

| Section | Actions |
|---------|---------|
| Navigation | Go to Library (1), Go to Reader (2), Go to Settings (3), Quit (q) |
| Library | Add Book (i), Jump Chapter (j), Update All (u), Delete (d), Cycle Filter (f), Cycle Status (Space) |
| Reader | Next Chapter (>), Prev Chapter (<), Toggle Paged/Scrollable |
| Settings | Open Settings (3), Toggle Debug Log, Plugin Configs |
| Debug | Reload Log, Clear Log |

**Interaction:** Type to fuzzy-filter, `↑/↓` navigate, `Enter` execute + close, `Esc` cancel.

---

## 4. Hints Toggle (`?`)

**Footer bar** at bottom of every screen, toggled with `?`

**Library:** `1 Library  2 Reader  3 Settings  i Add  j Jump  u Update  d Delete  f Filter  Space Status  : Palette  q Quit`

**Reader:** `> Next  < Prev  h/l Page  j/k Scroll  PgDn/PgUp  Esc Back  : Palette`

**Settings:** `↑/↓ Nav  Enter Select  Esc Back  : Palette`

**Modals:** Always show modal-specific hints (no toggle needed)

---

## 5. Consistency Rules

| Rule | Behavior |
|------|----------|
| `Esc` | Always closes modal / goes back one level / quits from Library |
| `Enter` | Confirms in modals, opens/activates in lists |
| `↑/↓` | Navigate everywhere |
| Numbers `1/2/3` | Always switch main screens |
| Letters | Contextual actions (vary by screen) |

---

## 6. Screen Keybindings

### Library (Screen 1)
| Key | Action |
|-----|--------|
| `1/2/3` | Switch screen |
| `i` | Add Book modal |
| `j` | Jump Chapter modal |
| `u` | Update All |
| `d` | Delete selected |
| `f` | Cycle Filter |
| `Space` | Cycle Status |
| `Enter` | Open book → Reader |
| `↑/↓` | Navigate list |
| `:` | Palette |
| `?` | Toggle hints |
| `q` / `Esc` | Quit |

### Reader (Screen 2)
| Key | Action |
|-----|--------|
| `1/2/3` | Switch screen |
| `>` / `Right` / `l` | Next chapter |
| `<` / `Left` / `h` | Prev chapter |
| `j` / `↓` | Scroll down / Next page |
| `k` / `↑` | Scroll up / Prev page |
| `PgDn` / `PgUp` | Page scroll |
| `Esc` | Back to Library |
| `:` | Palette |
| `?` | Toggle hints |

### Settings (Screen 3)
| Key | Action |
|-----|--------|
| `1/2/3` | Switch screen |
| `↑/↓` | Navigate list |
| `Enter` | Select / Enter submenu / Edit field |
| `Esc` | Back / Exit edit |
| `:` | Palette |
| `?` | Toggle hints |

### Modals
| Key | Action |
|-----|--------|
| `Esc` | Cancel / Close |
| `Enter` | Confirm / Save |
| `↑/↓` | Navigate |
| `Ctrl+S` | Submit (Add Book) |
| `t` | Toggle title/URL (Jump Chapter) |
| `Backspace` | Delete char/line |

---

## 7. Visual Layout Updates

### Library
- Keep 45/55 split (list/details)
- Filter bar: `Filter: Reading  [f] cycle`
- Footer hints bar (toggled)

### Reader
- Header: `Book Title — Ch.N Chapter Title`
- Footer hints (toggled)
- Page indicator in paged mode: `Page X/Y`

### Settings
- List of sections with sub-screens
- Footer hints (toggled)

### Modals
- Centered, 70% × 50%
- Title shows count
- Hints always visible

---

## 8. Implementation Scope

### Modified Files
1. `src/input/mod.rs` — Global key dispatch
2. `src/input/library.rs` — Updated bindings
3. `src/input/reader.rs` — Updated bindings
4. `src/input/settings.rs` — Updated bindings
5. `src/input/modal.rs` — Updated bindings
6. `src/ui/mod.rs` — Pass hints state
7. `src/ui/library.rs` — Footer hints rendering
8. `src/ui/reader.rs` — Footer hints rendering
9. `src/ui/settings.rs` — Footer hints rendering
10. `src/ui/modal.rs` — Modal hints
11. `src/state/mod.rs` — Add `show_hints: bool` to AppState
12. `src/app.rs` — Handle global keys in main loop

### New File
- `src/ui/palette.rs` — Command palette component

---

## 9. Command Palette Component

**`src/ui/palette.rs`**

```rust
struct CommandPalette {
    query: String,
    filtered: Vec<Action>,
    selected: usize,
    visible: bool,
}

struct Action {
    category: &'static str,
    label: &'static str,
    keys: &'static str,
    handler: fn(&mut AppState, &Sender<AppCommand>),
}
```

**Methods:** `open()`, `close()`, `filter(query)`, `execute()`, `draw(frame, area)`

**Categories:** Navigation, Library, Reader, Settings, Debug

**Modal enum addition:**
```rust
Modal::CommandPalette { 
    query: String, 
    filtered: Vec<Action>, 
    selected: usize 
}
```

---

## 10. Success Criteria

- [ ] All current functionality preserved
- [ ] Global 1/2/3 navigation works from any screen
- [ ] Command palette opens with `:` and fuzzy-searches all actions
- [ ] `?` toggles footer hints on all screens
- [ ] `Esc` consistently backs out everywhere
- [ ] `Enter` consistently confirms/opens
- [ ] No regression in reader modes, modals, settings flow
- [ ] Tests pass