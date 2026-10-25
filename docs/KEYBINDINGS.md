# Keybindings

## Global

| Key | Action |
|-----|--------|
| `1` | Library |
| `2` | Reader (skip session picker, open most recent) |
| `8` | Jobs |
| `9` | Settings |
| `:` | Command Palette |
| `?` | Toggle Hints |
| `Esc` | Back / Quit / Close Modal |

## Library

| Key | Action |
|-----|--------|
| `i` | Add Book |
| `j` | Jump Chapter |
| `u` | Update All |
| `d` | Delete Book |
| `f` | Filter modal (name, tags, status, library, AI search) |
| `e` | Embed Chapters (pick chapters to embed) |
| `g` | Toggle AI book/grouped mode |
| `.` / `,` | Cycle AI genre |
| `c` | Clear AI ranking |
| `Space` | Cycle Status |
| `Enter` | Session Picker / AI chapter list |
| `↑/↓` | Navigate |

## Reader

| Key | Action |
|-----|--------|
| `>` | Next Chapter |
| `<` | Prev Chapter |
| `→` | Next Page (Paged mode) |
| `←` | Prev Page (Paged mode) |
| `↓` | Scroll Down (Scrollable mode) |
| `↑` | Scroll Up (Scrollable mode) |
| `s` | Session Picker |
| `Esc` | Quit |

## Settings

| Key | Action |
|-----|--------|
| `↑/↓` | Navigate |
| `Enter` | Select / Edit |
| `Esc` | Back (on a subpage) / Quit (on the main page) |

Settings opens five subpages: Rate Limit, Debug Log, Reader Mode, Plugins, and Server. `Enter` toggles or edits the highlighted field, `↑/↓` scrolls long lists, and `Enter` on a plugin in the list opens its config fields.

## Modals

| Key | Action |
|-----|--------|
| `Esc` | Cancel |
| `Enter` | Confirm / Save |
| `↑/↓` | Navigate |
| `Ctrl+S` | Submit (Add Book) |
| `t` | Toggle Title/URL (Jump Chapter) |
| `n` | New Session |
| `r` | Rename Session |
| `d` | Delete Session |
| `Tab` | Cycle filter row / expand AI results |
| `Space` | Toggle tag / select (filter modal); toggle chapter (embed modal) |
| `c` | Clear filter facets / clear selection (embed modal) |
| `a` | Fetch all chapters (AI chapter list modal) / select all (embed modal) |

## Jobs

| Key | Action |
|-----|--------|
| `a` / `s` / `d` / `e` | Filter: All / Running / Completed / Failed |
| `Enter` | Expand job details |
| `c` / `C` | Cancel / Cancel All |
| `r` / `R` | Retry / Retry All Failed |
| `f` / `F` | Flush Completed / Flush All |
| `+` / `-` | Increase / Decrease Workers |

## Command palette

The palette (`:`) runs every screen action without memorizing keys:

- Navigation: Go to Library, Go to Reader, Go to Settings
- Library: Add Book, Jump Chapter, Embed Chapters…, Update All, Delete Book, Filter Library…, AI Search…, Cycle Status
- Reading: Next Chapter, Previous Chapter, Toggle Paged/Scrollable
- App: Toggle Debug Log, Backends…, Plugin Configs, Install Plugin from GitHub, Reload Log
