
# Scylla Reader

A TUI reader that interfaces with WebAssembly plugins to let you scrape, manage, and read web novels from the terminal. Progress is stored persistently in a local database.

![App Screenshot](extra/demo1.gif)

Multiple reading modes:
![App Screenshot](extra/demo2.gif)

## Layout

The project contains multiple crates:

- **`scylla-reader/`**: The TUI application.
- **`scylla-core/`**: Shared library — common types, scraper, worker, and messenger.
- **`scylla-server/`**: REST API server backed by the SQLite database.
- **`scylla-plugin-api/`**: Type definitions for plugin developers.

Plugins live in the separate [scylla-plugin-base](https://github.com/MothInBox/scylla-plugin-base) repository.

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
| `L` | Backends (add / rename / delete) |
| `Space` | Cycle Status |
| `Enter` | Session Picker |
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
- **AI Classification Model** — books are embedded locally (all-MiniLM-L6-v2) on chapter fetch and book creation, with genre classification and semantic search across chapters.
- **Plugin download from GitHub** — install plugins from a GitHub repo's latest release via the command palette.

### Planned

- **Plugin explore feed** — in-app browser for discovering books. Plugins expose a `search(query) -> SearchResults` function. TUI renders results, user picks one, then `scrape_book` runs. Start with search input + paginated results.
- **Customizable file paths** — add `data_dir`, `config_dir`, `plugin_dir` to `PersistedSettings`. `config_dir()` checks these overrides before `dirs`-based defaults.
