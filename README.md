
# Scylla Reader

A TUI reader that interfaces with WebAssembly plugins to let you scrape, manage, and read web novels from the terminal. Progress is stored persistently in a local database.

![App Screenshot](extra/demo1.gif)

Multiple reading modes:
![App Screenshot](extra/demo2.gif)

## Layout

The project contains multiple crates:

- **`scylla-reader/`**: The TUI application.
- **`scylla-core/`**: Shared library — common types, job worker, and messenger.
- **`scylla-server/`**: REST API server backed by the SQLite database — owns all jobs (scraping, chapter fetch, covers, embedding) and AI search, streaming events to the TUI over SSE.
- **`scylla-plugin-api/`**: Type definitions for plugin developers.

Plugins live in the separate [scylla-plugin-base](https://github.com/MothInBox/scylla-plugin-base) repository.

## AI Search

Press `f` in the library and enter a query to run an AI search. Books are ranked semantically using bge-small-en-v1.5 embeddings (chunked chapter indexing), cross-encoder reranking, and a BM25 hybrid — with a per-book diversity cap and 0–100 scores.

- `Enter` on a ranked book drills into its top chapters instantly (no round-trip); `a` fetches the full per-book ranking
- `g` toggles between ranked books and grouped chapters
- `.` / `,` cycle the genre facet; `Esc` clears the AI ranking
- The filter bar shows the active query, result count, and embedding coverage
- The first search downloads the models (~224MB) and may take a moment; a fresh library shows a hint instead of an error

## Installation

### Option A: Using Nix (Recommended)

#### Temporary

Run once:
```bash
nix run github:MothInBox/scylla-reader
```

Add to your shell temporarily:
```bash
nix shell github:MothInBox/scylla-reader
```

#### Declarative (flake.nix + home.nix)

In your flake inputs:
```nix
scylla-reader = {
    url = "github:MothInBox/scylla-reader";
    inputs.nixpkgs.follows = "nixpkgs";
};
```

Pass `scylla-reader` as a special arg to home-manager, then add to `home.packages`:
```nix
scylla-reader.packages.${pkgs.system}.default
```

### Option B: Using Cargo

**Prerequisites:** Rust stable toolchain, development headers for SSL and curl.

```bash
# Ubuntu/Debian
sudo apt install build-essential pkg-config libssl-dev libcurl4-openssl-dev

# Fedora
sudo dnf groupinstall "Development Tools" && sudo dnf install pkg-config openssl-dev libcurl-devel

# Arch
sudo pacman -S base-devel pkg-config openssl curl

# macOS
brew install openssl curl pkg-config
```

**Compile:**
```bash
git clone https://github.com/MothInBox/scylla-reader.git
cd scylla-reader/scylla-reader
cargo install --path .
```

## Keybindings

### Global
| Key | Action |
|-----|--------|
| `1` | Library |
| `2` | Reader (skip session picker, open most recent) |
| `8` | Jobs |
| `9` | Settings |
| `:` | Command Palette |
| `?` | Toggle Hints |
| `Esc` | Back / Quit / Close Modal |

### Library
| Key | Action |
|-----|--------|
| `i` | Add Book |
| `j` | Jump Chapter |
| `u` | Update All |
| `d` | Delete Book |
| `f` | Filter modal (name, tags, status, library, AI search) |
| `e` | Embed Chapters (pick chapters to embed) |
| `g` | Toggle AI book/grouped mode |
| `.` / `,` | Cycle AI genre facet |
| `Space` | Cycle Status |
| `Enter` | Session Picker / AI drill-down |
| `↑/↓` | Navigate |

### Reader
| Key | Action |
|-----|--------|
| `>` | Next Chapter |
| `<` | Prev Chapter |
| `→` | Next Page (Paged mode) |
| `←` | Prev Page (Paged mode) |
| `↓` | Scroll Down (Scrollable mode) |
| `↑` | Scroll Up (Scrollable mode) |
| `s` | Session Picker |
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
| `t` | Toggle Title/URL (Jump Chapter) |
| `n` | New Session |
| `r` | Rename Session |
| `d` | Delete Session |
| `Tab` | Switch filter facet / expand AI results |
| `Space` | Toggle tag / select (filter modal) |
| `c` | Clear filter facets |
| `a` | Fetch all chapters (AI drill-down modal) |

### Jobs
| Key | Action |
|-----|--------|
| `c` / `C` | Cancel / Cancel All |
| `r` / `R` | Retry / Retry All Failed |
| `f` / `F` | Flush Completed / Flush All |
| `+` / `-` | Increase / Decrease Workers |

## FAQ

### Where can I get plugins?

Develop your own or find one someone else has written.

See the [scylla-plugin-base](https://github.com/MothInBox/scylla-plugin-base) repository for the reference template and existing plugins.

> [!WARNING]
> Plugins run arbitrary wasm on your machine. Verify the plugin yourself or only use trusted sources.

### How do I install a plugin from GitHub?

Open the command palette (`:`), select "Install Plugin from GitHub", and enter a GitHub repo URL like `https://github.com/owner/repo`.

Scylla fetches the latest release from GitHub, scans it for `.wasm` files named `plugin-<domain>.wasm`, and installs them. The domain is extracted from the filename — e.g., `plugin-example.com.wasm` registers for `example.com`.

### How do I make a plugin repo?

See the [scylla-plugin-base](https://github.com/MothInBox/scylla-plugin-base) repository — it contains the reference template, the shared plugin API, and a full guide on writing, building, and releasing plugins.

### Where is data stored?

| Directory | Contents |
|-----------|----------|
| `~/.local/share/scylla-reader/` | `library.db` (database), `server.log`, `models/` (embedding model cache) |
| `~/.config/scylla-reader/` | `settings.json`, `libraries.json`, `plugin-*.json` (config) |
| `~/.config/scylla-reader/plugins/` | `plugin-*.wasm` (plugin binaries) |

## Roadmap

### Recently Completed
- Multiple reading sessions with names
- Persistent settings (rate limit, debug logging, reader mode)
- Word-wrap extracted to separate module
- Plugin config files now use `plugin-` prefix (no collision with `settings.json`)
- Domain matching fixed to use host segments instead of naive substring match
- Plugin cache uses `Mutex` (thread-safe across async boundaries)
- Curl requests have a 30s timeout
- Shared cookie-parsing helper
- Test setup deduplicated across 10 modules
- **HTTPS server** — `scylla-server` crate exposes a REST API for books, chapters, sessions, and settings, backed by SQLite.
- **Scylla as server client** — the TUI runs as a client of the server API through the `StorageBackend` abstraction.
- **Server-owned jobs** — the server owns scraping, chapter fetch, covers, embedding, and plugins; the TUI is a thin client fed by SSE.
- **AI search** — semantic search across books and chapters with bge-small-en-v1.5 embeddings, chunked chapter indexing, cross-encoder reranking, and BM25 hybrid retrieval.
- **Book-mode AI search** — AI-ranked library with a visible indicator, instant per-book drill-down, genre facet, and result snippets.
- **Plugin download from GitHub** — install plugins from a GitHub repo's latest release via the command palette.
- **Customizable file paths** — `data_dir`, `config_dir`, `plugin_dir` overrides in settings.
- **Code simplification pass** — deduplicated test helpers, job command handling, and loaders; linear-time AI ranking and grouping.

### Planned

- **Plugin explore feed** — in-app browser for discovering books. Plugins expose a `search(query) -> SearchResults` function. TUI renders results, user picks one, then `scrape_book` runs. Start with search input + paginated results.
- **Manual plugin selection** — when adding a book, if multiple plugins match the domain, show a warning and block submission until the conflict is resolved; also lets the user choose which plugin/source to use.
- **Generic modals for data entry & prompts** — a shared modal system for future text entry and prompts 
- **Design unification** — a pass to unify the visual language across pages and modals (consistent spacing, hierarchy, and component styles).
- **README demo media** — update the demo gifs and images to showcase the new features (AI search, drill-down, genre facet, snippets).
- **Per-plugin scraping delay** — move the scraping rate limit from a single global setting to a per-plugin (per-domain) delay, adjustable via settings.
