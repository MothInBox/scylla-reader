# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Server-owned jobs with SSE streaming — the server owns scraping, chapter fetching, covers, embedding, and plugins; the TUI is a thin client
- AI search: server-side embeddings (all-MiniLM-L6-v2 via candle) with book and chapter search modes
- EmbedBatch job with per-chapter detail, honest statuses (Pending/Fetched/Embedded), n/x/m progress, two-phase ETA, and autoembed setting
- Plugin install as a server job with a shared registry (no restart required)
- Library management: add/edit/remove libraries with backend picker
- Library filter modal (name/tag/status/library search)
- Settings editing with inline edit buffer and cursor
- Command palette actions for embed, backends, and AI search
- Connection state surfacing (connected/error) with snapshot retry
- ChapterResults collapse/expand (Tab)
- AI search first-run hint when no chapters are embedded yet (no error toast)
- Per-book result diversity (top-3 per book) and 0–100 normalized scores with a relevance floor in AI search
- Book-mode AI search: the library re-ranks with an explicit AI indicator (filter-bar segment, "AI ranked" title, mode-aware footer hints) and per-row scores
- AI drill-down: Enter on a ranked book opens its inline chapters (local, no network); `g` toggles book-mode ↔ grouped-chapter mode; Esc clears the AI ranking
- AI state is explicit and survives filter commits and deletes — clearing is always intentional (Esc)
- Chapter embeddings are stored per 512-token chunk (with text) and search uses each chapter's best chunk; coverage counts are distinct chapters, not chunks
- Hybrid search: BM25 keyword lane fused with the semantic lane via Reciprocal Rank Fusion, then refined by a cross-encoder reranker (ms-marco-MiniLM-L-6-v2)
- Embedding model upgraded to bge-small-en-v1.5 with a query/passage instruction split; a model change clears stored embeddings and re-embeds explicitly (never silent)

### Changed

- Server is built in release mode (`just run`)
- plugin-template moved to the separate `scylla-plugin-base` repo
- Embeddings are cascade-deleted when a book is deleted
- AI search chapter scores are normalized to a 0–100 display scale
- Content hashes are stable FNV-1a and include the embedding model version, so a model swap re-embeds unchanged chapters exactly once
- Search embed + rank + rerank run on the blocking pool (`spawn_blocking`) so the tokio worker never stalls

### Fixed

- Embed ETA and detail clobbering
- Settings editing visibility and global-key suppression while editing
- Test config race corrupting `libraries.json`
- Snapshot retry and connection surfacing

## [0.2.0] - 2026-07-05

### Added

- Extism WASM plugin system with config schemas and host functions (`host_curl_fetch`, `host_scylla_fail`)
- Plugin install from GitHub releases via the command palette
- Jobs page with worker management, filters, retry, cancel, and per-domain rate limiting
- Customizable file paths via `settings.json`
- Cover caching
- Job outcomes with expanded detail view