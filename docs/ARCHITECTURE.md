# Architecture

Scylla Reader runs as two processes: the TUI you interact with, and a server that owns the data and the work. The TUI holds no database and runs no jobs.

```
 scylla-reader (TUI)
       |
       |  HTTP requests + SSE event stream
       v
 scylla-server (REST API, SQLite, job workers, embeddings)
       |
       |  Extism host calls
       v
 plugin-<domain>.wasm  ----  target website
```

## Crates

| Crate | Role |
|-------|------|
| `scylla-reader` | The TUI. Renders screens, sends requests, applies SSE events. |
| `scylla-server` | REST API, SQLite storage, job workers, and embeddings. |
| `scylla-core` | Shared types, paths, worker, messaging, plugin loading and install used by both. |
| `scylla-plugin-api` | Types a plugin compiles against. |

## The split

The server listens on `127.0.0.1:8080` by default (`scylla-server [port]` changes it) and owns everything with side effects: scraping, chapter fetching, cover downloads, embedding, and plugin installs. Jobs run server-side with a worker pool you can resize from the Jobs screen (`+` / `-`).

The TUI talks to the server over HTTP for commands and subscribes to an SSE stream for state. Job state, setting changes, and new chapters arrive as events; the TUI does not poll for them. The one exception is the per-book embedding-status indicator, which the TUI re-fetches every 10 seconds. If the connection drops, the TUI shows it and retries its snapshot.

Threads inside the server and the TUI pass job commands and events over channels. The handles the HTTP handlers need, the database, the job list, and the plugin registry, sit behind mutexes.

## Storage

SQLite in WAL mode at `~/.local/share/scylla-reader/library.db`. The server reads and writes it exclusively. See [CONFIGURATION.md](CONFIGURATION.md) for every path the project touches.

## Search

Embeddings and keyword search live in the server, built on candle. The TUI sends a query and renders what comes back; it never loads a model or embeds text itself. The first search downloads the models (~224MB) into `~/.local/share/scylla-reader/models/`.

## Plugins

The server loads `.wasm` plugins through Extism and calls `scrape_book` and `scrape_chapter`. A plugin reaches the network only through the `curl_fetch` host function, so every request the project makes is visible in one place. Plugin source lives in [scylla-plugin-base](https://github.com/MothInBox/scylla-plugin-base); see [PLUGINS.md](PLUGINS.md).
