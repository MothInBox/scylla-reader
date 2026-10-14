# Configuration

## Storage paths

| Directory | Contents |
|-----------|----------|
| `~/.local/share/scylla-reader/` | `library.db` (database), `server.log`, `models/` (embedding model cache) |
| `~/.local/state/scylla-reader/` | `scylla-reader.log` (TUI log, written when debug logging is on) |
| `~/.config/scylla-reader/` | `settings.json`, `libraries.json`, `plugin-*.json` (per-plugin config) |
| `~/.config/scylla-reader/plugins/` | `plugin-*.wasm` (plugin binaries) |

## settings.json

Scylla writes this file when you change a setting: most keys from the Settings screen (`9`), and `max_workers` from the Jobs screen (`8`). The keys:

| Key | Values | Meaning |
|-----|--------|---------|
| `rate_limit_secs` | seconds | Delay between scraping requests |
| `debug_log` | bool | Verbose TUI logging into `~/.local/state/scylla-reader/scylla-reader.log` |
| `reader_mode` | `"Paged"` / `"Scrollable"` | Page keys or scrolling |
| `max_workers` | 1-32 | Concurrent job workers (the Jobs screen clamps here; the API accepts up to 255) |
| `config_dir` | path | Override where config files live |
| `data_dir` | path | Override where the database lives |

`config_dir` and `data_dir` appear only after you set an override; delete the keys to return to the default paths listed above. Restart the app after editing the file by hand.

The server process writes `server.log` in the data directory on its own; that log is always on and independent of `debug_log`.

## Settings that live outside the file

The Auto-Embed New Chapters toggle on the Settings screen is server-side state kept in memory. It resets when the server restarts. The Server Connection entry shows whether the TUI can reach the server.

## Backends

`libraries.json` holds the servers the TUI can talk to. The command palette's Backends picker (`:` then Backends…) adds a connection and switches between them, so you can point the TUI at a server running on another port or machine.

## Command line

```sh
scylla-reader        # the TUI
scylla-server 9000   # the server on another port (default 8080)
```

`just run` builds the server in release mode and starts the TUI against it.
