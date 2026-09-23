#![allow(dead_code)]

use rusqlite::{Connection, OptionalExtension, Result, params};
use scylla_core::types::{Book, BookStatus, Chapter, Progress, Session};
use std::collections::HashMap;

/// Stored book embedding row: (description, aggregate, genres).
pub type BookEmbedding = (Option<Vec<f32>>, Option<Vec<f32>>, Option<Vec<String>>);

/// Per-book embedding status:
/// (embedded_chapters, total_chapters, has_aggregate, genres, embedded_chapter_urls).
pub type EmbeddingStatus = (usize, usize, bool, Option<Vec<String>>, Vec<String>);

pub struct ServerDb {
    conn: Connection,
}

impl ServerDb {
    pub fn open(path: &std::path::Path) -> Result<Self> {
        Self::open_conn(Connection::open(path)?)
    }

    pub fn open_path(path: &std::path::Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        Self::open_conn(Connection::open(path)?)
    }

    pub fn open_conn(conn: Connection) -> Result<Self> {
        conn.execute_batch(
            "
            PRAGMA foreign_keys = ON;
            PRAGMA journal_mode = WAL;
            PRAGMA synchronous = NORMAL;
        ",
        )?;
        let db = Self { conn };
        db.migrate()?;
        Ok(db)
    }
}

impl ServerDb {
    fn migrate(&self) -> Result<()> {
        self.conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS books (
                url               TEXT PRIMARY KEY,
                title             TEXT NOT NULL,
                status            TEXT NOT NULL DEFAULT 'Reading',
                active_session_id INTEGER,
                cover_url         TEXT,
                description       TEXT
            );

            CREATE TABLE IF NOT EXISTS sessions (
                id         INTEGER PRIMARY KEY AUTOINCREMENT,
                book_url   TEXT NOT NULL REFERENCES books(url) ON DELETE CASCADE,
                name       TEXT NOT NULL,
                current    INTEGER NOT NULL DEFAULT 0,
                total      INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE TABLE IF NOT EXISTS tags (
                book_url TEXT NOT NULL REFERENCES books(url) ON DELETE CASCADE,
                tag      TEXT NOT NULL,
                PRIMARY KEY (book_url, tag)
            );

            CREATE TABLE IF NOT EXISTS chapters (
                book_url TEXT NOT NULL REFERENCES books(url) ON DELETE CASCADE,
                url      TEXT NOT NULL,
                title    TEXT NOT NULL,
                ord      INTEGER NOT NULL,
                PRIMARY KEY (book_url, url)
            );

            CREATE TABLE IF NOT EXISTS book_embeddings (
                book_url              TEXT PRIMARY KEY,
                description_embedding BLOB,
                aggregate_embedding   BLOB,
                aggregate_updated_at  INTEGER,
                genres                TEXT
            );

            CREATE TABLE IF NOT EXISTS meta (
                key   TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
        ",
        )?;

        // `chapter_embeddings` is per-chunk now. The legacy single-row schema
        // (source_url PK, no chunk_idx/text) is dropped: its rows lack chunk
        // text and were produced by the old model, so they must be re-embedded
        // anyway. The no_embeddings hint surfaces the re-embed requirement.
        self.migrate_chapter_embeddings()?;

        // Model identity: if the stored embedding model differs from the
        // configured one, every stored vector is in a different space — clear
        // both embedding tables so the re-embed is explicit, never silent.
        self.migrate_embedding_model()?;

        let _ = self
            .conn
            .execute_batch("ALTER TABLE books ADD COLUMN active_session_id INTEGER;");

        let _ = self.conn.execute(
            "INSERT INTO sessions (book_url, name, current, total)
             SELECT url, 'Initial', current, total FROM books
             WHERE NOT EXISTS (SELECT 1 FROM sessions WHERE sessions.book_url = books.url)",
            [],
        );

        let _ = self.conn.execute(
            "UPDATE books SET active_session_id = (
                SELECT id FROM sessions
                WHERE sessions.book_url = books.url
                ORDER BY updated_at DESC LIMIT 1
             ) WHERE active_session_id IS NULL",
            [],
        );

        Ok(())
    }

    /// Creates the per-chunk `chapter_embeddings` table, dropping the legacy
    /// single-row schema first if present.
    fn migrate_chapter_embeddings(&self) -> Result<()> {
        if self.table_exists("chapter_embeddings")?
            && !self.column_exists("chapter_embeddings", "chunk_idx")?
        {
            self.conn.execute_batch("DROP TABLE chapter_embeddings;")?;
            eprintln!(
                "MIGRATE: dropped legacy single-row chapter_embeddings \
                 (pre-chunking schema); re-embedding required"
            );
        }
        self.conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS chapter_embeddings (
                chapter_url  TEXT NOT NULL,
                chunk_idx    INTEGER NOT NULL,
                book_url     TEXT,
                embedding    BLOB NOT NULL,
                text         TEXT NOT NULL,
                embedded_at  INTEGER NOT NULL,
                content_hash TEXT,
                PRIMARY KEY (chapter_url, chunk_idx)
            );

            CREATE INDEX IF NOT EXISTS idx_chapter_embeddings_chapter_url
                ON chapter_embeddings(chapter_url);
            CREATE INDEX IF NOT EXISTS idx_chapter_embeddings_book_url
                ON chapter_embeddings(book_url);
        ",
        )?;
        Ok(())
    }

    fn table_exists(&self, name: &str) -> Result<bool> {
        self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
            [name],
            |row| row.get(0),
        )
    }

    fn column_exists(&self, table: &str, column: &str) -> Result<bool> {
        let mut stmt = self.conn.prepare(&format!("PRAGMA table_info({table})"))?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let name: String = row.get(1)?;
            if name == column {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Clears all stored embeddings when the model that produced them differs
    /// from the configured one. Vectors from different models live in different
    /// spaces — cosine across models is garbage — so the swap must be explicit
    /// and visible, never silent. The `meta` table records the model that
    /// produced the current rows.
    fn migrate_embedding_model(&self) -> Result<()> {
        let stored: Option<String> = self
            .conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'embedding_model'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        if stored.as_deref() == Some(crate::embeddings::MODEL_VERSION) {
            return Ok(());
        }
        self.conn.execute_batch(
            "DELETE FROM chapter_embeddings;
             DELETE FROM book_embeddings;",
        )?;
        self.conn.execute(
            "INSERT INTO meta (key, value) VALUES ('embedding_model', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [crate::embeddings::MODEL_VERSION],
        )?;
        eprintln!(
            "MIGRATE: embedding model changed ({} -> {}); cleared chapter and \
             book embeddings — re-embedding required",
            stored.as_deref().unwrap_or("none"),
            crate::embeddings::MODEL_VERSION
        );
        Ok(())
    }
}

impl ServerDb {
    pub fn load_books(&self) -> Result<Vec<Book>> {
        let mut stmt = self.conn.prepare(
            "SELECT url, title, status, active_session_id, cover_url, description
             FROM books ORDER BY rowid",
        )?;

        let mut books: Vec<Book> = stmt
            .query_map([], |row| {
                let status_str: String = row.get(2)?;
                Ok(Book {
                    url: row.get(0)?,
                    title: row.get(1)?,
                    status: parse_status(&status_str),
                    active_session_id: row.get(3)?,
                    cover_url: row.get(4)?,
                    description: row.get(5)?,
                    tags: Vec::new(),
                    chapters: Vec::new(),
                    sessions: Vec::new(),
                })
            })?
            .collect::<Result<_>>()?;

        for book in &mut books {
            book.tags = self.load_tags(&book.url)?;
            book.chapters = self.load_chapters(&book.url)?;
            book.sessions = self.load_sessions_for_book(&book.url)?;
        }

        Ok(books)
    }

    fn load_tags(&self, book_url: &str) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT tag FROM tags WHERE book_url = ? ORDER BY tag")?;
        stmt.query_map([book_url], |row| row.get(0))?.collect()
    }

    pub fn load_chapters(&self, book_url: &str) -> Result<Vec<Chapter>> {
        let mut stmt = self
            .conn
            .prepare("SELECT url, title, ord FROM chapters WHERE book_url = ? ORDER BY ord")?;
        stmt.query_map([book_url], |row| {
            Ok(Chapter {
                url: row.get(0)?,
                title: row.get(1)?,
                order: row.get(2)?,
            })
        })?
        .collect()
    }

    pub fn load_sessions_for_book(&self, book_url: &str) -> Result<Vec<Session>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, book_url, name, current, total, created_at, updated_at
             FROM sessions WHERE book_url = ? ORDER BY updated_at DESC",
        )?;
        stmt.query_map([book_url], |row| {
            Ok(Session {
                id: row.get(0)?,
                book_url: row.get(1)?,
                name: row.get(2)?,
                progress: Progress {
                    current: row.get(3)?,
                    total: row.get(4)?,
                },
                created_at: row.get(5)?,
                updated_at: row.get(6)?,
            })
        })?
        .collect()
    }
}

impl ServerDb {
    pub fn upsert_book(&self, book: &Book) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;

        tx.execute(
            "INSERT INTO books (url, title, status, active_session_id, cover_url, description)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(url) DO UPDATE SET
               title             = excluded.title,
               status            = CASE
                 WHEN excluded.status = 'Reading' AND books.status != 'Reading' THEN books.status
                 ELSE excluded.status
               END,
               active_session_id = CASE
                 WHEN excluded.active_session_id IS NULL THEN books.active_session_id
                 ELSE excluded.active_session_id
               END,
               cover_url         = excluded.cover_url,
               description       = excluded.description",
            params![
                book.url,
                book.title,
                status_str(&book.status),
                book.active_session_id,
                book.cover_url,
                book.description,
            ],
        )?;

        if !book.tags.is_empty() {
            tx.execute("DELETE FROM tags WHERE book_url = ?", [&book.url])?;
            {
                let mut stmt =
                    tx.prepare("INSERT OR IGNORE INTO tags (book_url, tag) VALUES (?1, ?2)")?;
                for tag in &book.tags {
                    stmt.execute(params![book.url, tag])?;
                }
            }
        }

        tx.execute("DELETE FROM chapters WHERE book_url = ?", [&book.url])?;
        {
            let mut stmt = tx.prepare(
                "INSERT OR IGNORE INTO chapters (book_url, url, title, ord)
                 VALUES (?1, ?2, ?3, ?4)",
            )?;
            for ch in &book.chapters {
                stmt.execute(params![book.url, ch.url, ch.title, ch.order])?;
            }
        }

        tx.commit()?;

        self.ensure_default_session(&book.url, book.chapters.len() as u32)?;
        Ok(())
    }

    pub fn delete_book(&self, book_url: &str) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        // The embeddings tables have no FK to books, so they must be cleaned up
        // explicitly (chapters/tags/sessions cascade via ON DELETE CASCADE).
        tx.execute(
            "DELETE FROM chapter_embeddings WHERE book_url = ?",
            [book_url],
        )?;
        tx.execute("DELETE FROM book_embeddings WHERE book_url = ?", [book_url])?;
        tx.execute("DELETE FROM books WHERE url = ?", [book_url])?;
        tx.commit()?;
        Ok(())
    }

    pub fn update_status(&self, book_url: &str, status: &BookStatus) -> Result<()> {
        self.conn.execute(
            "UPDATE books SET status = ?1 WHERE url = ?2",
            params![status_str(status), book_url],
        )?;
        Ok(())
    }
}

impl ServerDb {
    pub fn create_session(&self, book_url: &str, name: &str, total: u32) -> Result<Session> {
        self.conn.execute(
            "INSERT INTO sessions (book_url, name, current, total) VALUES (?1, ?2, 0, ?3)",
            params![book_url, name, total],
        )?;
        Ok(Session {
            id: self.conn.last_insert_rowid(),
            book_url: book_url.to_string(),
            name: name.to_string(),
            progress: Progress { current: 0, total },
            created_at: String::new(),
            updated_at: String::new(),
        })
    }

    pub fn rename_session(&self, id: i64, name: &str) -> Result<usize> {
        self.conn.execute(
            "UPDATE sessions SET name = ?1, updated_at = datetime('now') WHERE id = ?2",
            params![name, id],
        )
    }

    pub fn delete_session(&self, id: i64) -> Result<usize> {
        let affected = self
            .conn
            .execute("DELETE FROM sessions WHERE id = ?", [id])?;
        if affected > 0 {
            self.conn.execute(
                "UPDATE books SET active_session_id = NULL WHERE active_session_id = ?",
                [id],
            )?;
        }
        Ok(affected)
    }

    pub fn update_session_progress(&self, id: i64, current: u32) -> Result<()> {
        self.conn.execute(
            "UPDATE sessions SET current = ?1, updated_at = datetime('now') WHERE id = ?2",
            params![current, id],
        )?;
        Ok(())
    }

    pub fn set_active_session(&self, book_url: &str, session_id: Option<i64>) -> Result<()> {
        self.conn.execute(
            "UPDATE books SET active_session_id = ?1 WHERE url = ?2",
            params![session_id, book_url],
        )?;
        if let Some(id) = session_id {
            self.conn.execute(
                "UPDATE sessions SET updated_at = datetime('now') WHERE id = ?1",
                params![id],
            )?;
        }
        Ok(())
    }

    pub fn ensure_default_session(&self, book_url: &str, total: u32) -> Result<i64> {
        let existing = self.load_sessions_for_book(book_url)?;
        if let Some(s) = existing.first() {
            return Ok(s.id);
        }
        let session = self.create_session(book_url, "Initial", total)?;
        Ok(session.id)
    }
}

impl ServerDb {
    /// Atomically replaces all chunks of a chapter. `chunks` is
    /// `(chunk_idx, text, embedding)`; a failed chunk marks the whole chapter
    /// failed (nothing is written).
    pub fn upsert_chapter_chunks(
        &self,
        chapter_url: &str,
        book_url: Option<&str>,
        chunks: &[(usize, &str, &[f32])],
        content_hash: Option<&str>,
    ) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "DELETE FROM chapter_embeddings WHERE chapter_url = ?",
            [chapter_url],
        )?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO chapter_embeddings
                     (chapter_url, chunk_idx, book_url, embedding, text, embedded_at, content_hash)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )?;
            let now = now_unix_ms();
            for (idx, text, emb) in chunks {
                stmt.execute(params![
                    chapter_url,
                    *idx as i64,
                    book_url,
                    encode_f32s(emb),
                    text,
                    now,
                    content_hash,
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// (chunk_count, content_hash) for a chapter — used to skip re-embedding
    /// unchanged chapters (all chunks share the chapter's content hash).
    pub fn chapter_chunk_state(&self, chapter_url: &str) -> Result<(usize, Option<String>)> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM chapter_embeddings WHERE chapter_url = ?",
            [chapter_url],
            |row| row.get(0),
        )?;
        let hash: Option<Option<String>> = self
            .conn
            .query_row(
                "SELECT content_hash FROM chapter_embeddings WHERE chapter_url = ? LIMIT 1",
                [chapter_url],
                |row| row.get(0),
            )
            .optional()?;
        Ok((count as usize, hash.flatten()))
    }

    pub fn upsert_book_embedding(
        &self,
        book_url: &str,
        description_embedding: Option<&[f32]>,
        aggregate_embedding: Option<&[f32]>,
        genres: Option<&[String]>,
    ) -> Result<()> {
        let desc_blob = description_embedding.map(encode_f32s);
        let agg_blob = aggregate_embedding.map(encode_f32s);
        let genres_json = genres.map(|g| serde_json::to_string(g).unwrap_or_else(|_| "[]".into()));
        let agg_ts = if aggregate_embedding.is_some() {
            Some(now_unix_ms())
        } else {
            None
        };
        self.conn.execute(
            "INSERT INTO book_embeddings (book_url, description_embedding, aggregate_embedding, aggregate_updated_at, genres)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(book_url) DO UPDATE SET
               description_embedding = COALESCE(excluded.description_embedding, book_embeddings.description_embedding),
               aggregate_embedding   = COALESCE(excluded.aggregate_embedding, book_embeddings.aggregate_embedding),
               aggregate_updated_at  = COALESCE(excluded.aggregate_updated_at, book_embeddings.aggregate_updated_at),
               genres                = COALESCE(excluded.genres, book_embeddings.genres)",
            params![book_url, desc_blob, agg_blob, agg_ts, genres_json],
        )?;
        Ok(())
    }

    pub fn get_book_embedding(&self, book_url: &str) -> Result<Option<BookEmbedding>> {
        let mut stmt = self.conn.prepare(
            "SELECT description_embedding, aggregate_embedding, genres
             FROM book_embeddings WHERE book_url = ?",
        )?;
        let mut rows = stmt.query_map([book_url], |row| {
            let desc: Option<Vec<u8>> = row.get(0)?;
            let agg: Option<Vec<u8>> = row.get(1)?;
            let genres: Option<String> = row.get(2)?;
            let genres = genres.and_then(|g| serde_json::from_str(&g).ok());
            Ok((
                desc.map(|b| decode_f32s(&b)),
                agg.map(|b| decode_f32s(&b)),
                genres,
            ))
        })?;
        rows.next().transpose()
    }

    /// One embedding per chapter for a book (chunk embeddings averaged per
    /// chapter — chapter-equal weighting for the book aggregate). Used by the
    /// aggregate recomputation.
    pub fn load_chapter_embeddings_for_book(&self, book_url: &str) -> Result<Vec<Vec<f32>>> {
        let mut stmt = self.conn.prepare(
            "SELECT chapter_url, embedding FROM chapter_embeddings
             WHERE book_url = ? ORDER BY chapter_url, chunk_idx",
        )?;
        let rows = stmt.query_map([book_url], |row| {
            let url: String = row.get(0)?;
            let blob: Vec<u8> = row.get(1)?;
            Ok((url, decode_f32s(&blob)))
        })?;
        let mut order: Vec<String> = Vec::new();
        let mut sums: HashMap<String, (Vec<f32>, usize)> = HashMap::new();
        for row in rows {
            let (url, emb) = row?;
            let entry = sums.entry(url.clone()).or_insert_with(|| {
                order.push(url.clone());
                (vec![0.0; emb.len()], 0)
            });
            if entry.0.len() == emb.len() {
                for (s, v) in entry.0.iter_mut().zip(&emb) {
                    *s += v;
                }
                entry.1 += 1;
            }
        }
        Ok(order
            .into_iter()
            .filter_map(|url| {
                sums.remove(&url).map(|(sum, n)| {
                    if n == 0 {
                        sum
                    } else {
                        sum.into_iter().map(|s| s / n as f32).collect()
                    }
                })
            })
            .collect())
    }

    pub fn find_book_url_for_chapter(&self, chapter_url: &str) -> Result<Option<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT book_url FROM chapters WHERE url = ? LIMIT 1")?;
        let mut rows = stmt.query_map([chapter_url], |row| row.get(0))?;
        rows.next().transpose()
    }

    /// Books that have a description but no description embedding yet (startup
    /// backfill candidates). Returns (book_url, description).
    pub fn books_missing_description_embedding(&self) -> Result<Vec<(String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT b.url, b.description FROM books b
             LEFT JOIN book_embeddings be ON be.book_url = b.url
             WHERE b.description IS NOT NULL AND be.description_embedding IS NULL",
        )?;
        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.collect()
    }

    /// All chunk embeddings enriched for search as `ChunkHit` rows. Chapters
    /// without a book_url map to empty strings / 0.
    pub fn load_all_chunk_hits(&self) -> Result<Vec<crate::embeddings::ChunkHit>> {
        let mut stmt = self.conn.prepare(
            "SELECT ce.book_url, b.title, ce.chapter_url, ch.title, ch.ord, be.genres,
                    ce.chunk_idx, ce.text, ce.embedding
             FROM chapter_embeddings ce
             LEFT JOIN chapters ch ON ch.book_url = ce.book_url AND ch.url = ce.chapter_url
             LEFT JOIN books b ON b.url = ce.book_url
             LEFT JOIN book_embeddings be ON be.book_url = ce.book_url
             ORDER BY ce.rowid",
        )?;
        let rows = stmt.query_map([], chunk_hit_from_row)?;
        rows.collect()
    }

    /// Chunk embeddings for one book, enriched for search (the per-book
    /// drill-down window).
    pub fn load_chunk_hits_for_book(
        &self,
        book_url: &str,
    ) -> Result<Vec<crate::embeddings::ChunkHit>> {
        let mut stmt = self.conn.prepare(
            "SELECT ce.book_url, b.title, ce.chapter_url, ch.title, ch.ord, be.genres,
                    ce.chunk_idx, ce.text, ce.embedding
             FROM chapter_embeddings ce
             LEFT JOIN chapters ch ON ch.book_url = ce.book_url AND ch.url = ce.chapter_url
             LEFT JOIN books b ON b.url = ce.book_url
             LEFT JOIN book_embeddings be ON be.book_url = ce.book_url
             WHERE ce.book_url = ?1
             ORDER BY ce.rowid",
        )?;
        let rows = stmt.query_map([book_url], chunk_hit_from_row)?;
        rows.collect()
    }

    /// Total number of chapters in the library (for the no-embeddings hint).
    pub fn chapter_count(&self) -> Result<usize> {
        let n: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM chapters", [], |row| row.get(0))?;
        Ok(n as usize)
    }

    /// Number of chapters with at least one embedded chunk (search coverage
    /// reporting). Counts DISTINCT chapters, not chunks.
    pub fn embedded_chapter_count(&self) -> Result<usize> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(DISTINCT chapter_url) FROM chapter_embeddings",
            [],
            |row| row.get(0),
        )?;
        Ok(n as usize)
    }

    /// All book aggregate embeddings as (book_url, aggregate, genres) — for
    /// search. Rows without an aggregate embedding are excluded.
    pub fn load_all_book_embeddings(&self) -> Result<Vec<crate::embeddings::BookAggregate>> {
        let mut stmt = self.conn.prepare(
            "SELECT book_url, aggregate_embedding, genres FROM book_embeddings
             WHERE aggregate_embedding IS NOT NULL ORDER BY rowid",
        )?;
        let rows = stmt.query_map([], |row| {
            let book_url: String = row.get(0)?;
            let agg: Vec<u8> = row.get(1)?;
            let genres: Option<String> = row.get(2)?;
            let genres = genres.and_then(|g| serde_json::from_str(&g).ok());
            Ok((book_url, decode_f32s(&agg), genres))
        })?;
        rows.collect()
    }

    /// All book aggregate embeddings with their titles, as
    /// (book_url, title, aggregate_embedding, genres) — for book-mode search
    /// hits that carry inline chapter data. Rows without an aggregate
    /// embedding are excluded; a missing books row maps the title to "".
    pub fn load_all_book_embeddings_with_titles(
        &self,
    ) -> Result<Vec<crate::embeddings::BookAggregateWithTitle>> {
        let mut stmt = self.conn.prepare(
            "SELECT be.book_url, b.title, be.aggregate_embedding, be.genres
             FROM book_embeddings be
             LEFT JOIN books b ON b.url = be.book_url
             WHERE be.aggregate_embedding IS NOT NULL ORDER BY be.rowid",
        )?;
        let rows = stmt.query_map([], |row| {
            let book_url: String = row.get(0)?;
            let title: Option<String> = row.get(1)?;
            let agg: Vec<u8> = row.get(2)?;
            let genres: Option<String> = row.get(3)?;
            let genres = genres.and_then(|g| serde_json::from_str(&g).ok());
            Ok((
                book_url,
                title.unwrap_or_default(),
                decode_f32s(&agg),
                genres,
            ))
        })?;
        rows.collect()
    }

    /// Per-book embedding status: (embedded_chapters, total_chapters,
    /// has_aggregate, genres, embedded_chapter_urls). Returns `None` if the
    /// book doesn't exist.
    pub fn embedding_status(&self, book_url: &str) -> Result<Option<EmbeddingStatus>> {
        let exists: bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM books WHERE url = ?)",
            [book_url],
            |row| row.get(0),
        )?;
        if !exists {
            return Ok(None);
        }
        let embedded_chapters: i64 = self.conn.query_row(
            "SELECT COUNT(DISTINCT chapter_url) FROM chapter_embeddings WHERE book_url = ?",
            [book_url],
            |row| row.get(0),
        )?;
        let total_chapters: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM chapters WHERE book_url = ?",
            [book_url],
            |row| row.get(0),
        )?;
        let (has_aggregate, genres): (bool, Option<String>) = self
            .conn
            .query_row(
                "SELECT aggregate_embedding IS NOT NULL, genres
                 FROM book_embeddings WHERE book_url = ?",
                [book_url],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?
            .unwrap_or((false, None));
        let genres = genres.and_then(|g| serde_json::from_str(&g).ok());
        let embedded_chapter_urls: Vec<String> = {
            let mut stmt = self.conn.prepare(
                "SELECT DISTINCT chapter_url FROM chapter_embeddings
                 WHERE book_url = ? ORDER BY chapter_url",
            )?;
            let rows = stmt.query_map([book_url], |row| row.get(0))?;
            rows.collect::<Result<_>>()?
        };
        Ok(Some((
            embedded_chapters as usize,
            total_chapters as usize,
            has_aggregate,
            genres,
            embedded_chapter_urls,
        )))
    }
}

fn status_str(s: &BookStatus) -> &'static str {
    match s {
        BookStatus::Reading => "Reading",
        BookStatus::Paused => "Paused",
        BookStatus::Dropped => "Dropped",
        BookStatus::Completed => "Completed",
    }
}

/// Maps a chunk-embeddings search row (book_url, book_title, chapter_url,
/// chapter_title, chapter_idx, genres, chunk_idx, text, embedding) into a
/// `ChunkHit`. Shared by the all-chunks and per-book loaders.
fn chunk_hit_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<crate::embeddings::ChunkHit> {
    let book_url: Option<String> = row.get(0)?;
    let book_title: Option<String> = row.get(1)?;
    let chapter_url: String = row.get(2)?;
    let chapter_title: Option<String> = row.get(3)?;
    let chapter_idx: Option<i64> = row.get(4)?;
    let genres: Option<String> = row.get(5)?;
    let chunk_idx: i64 = row.get(6)?;
    let text: String = row.get(7)?;
    let blob: Vec<u8> = row.get(8)?;
    let genres = genres.and_then(|g| serde_json::from_str(&g).ok());
    Ok(crate::embeddings::ChunkHit {
        book_url: book_url.unwrap_or_default(),
        book_title: book_title.unwrap_or_default(),
        chapter_url,
        chapter_title: chapter_title.unwrap_or_default(),
        chapter_idx: chapter_idx.unwrap_or(0) as u32,
        genres,
        chunk_idx: chunk_idx as u32,
        text,
        embedding: decode_f32s(&blob),
    })
}

/// Encodes an `f32` vector as raw little-endian bytes (BLOB storage).
fn encode_f32s(v: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 4);
    for f in v {
        out.extend_from_slice(&f.to_le_bytes());
    }
    out
}

/// Decodes a raw little-endian byte BLOB back into an `f32` vector.
fn decode_f32s(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

/// Current unix epoch time in milliseconds.
fn now_unix_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn parse_status(s: &str) -> BookStatus {
    match s {
        "Paused" => BookStatus::Paused,
        "Dropped" => BookStatus::Dropped,
        "Completed" => BookStatus::Completed,
        _ => BookStatus::Reading,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> ServerDb {
        let conn = Connection::open_in_memory().unwrap();
        ServerDb::open_conn(conn).unwrap()
    }

    fn sample_book(url: &str) -> Book {
        Book {
            title: format!("Book {}", url),
            url: url.to_string(),
            status: BookStatus::Reading,
            sessions: vec![],
            active_session_id: None,
            tags: vec!["tag1".into()],
            cover_url: None,
            description: Some("desc".into()),
            chapters: vec![
                Chapter {
                    url: "ch1".into(),
                    title: "Chapter 1".into(),
                    order: 1,
                },
                Chapter {
                    url: "ch2".into(),
                    title: "Chapter 2".into(),
                    order: 2,
                },
            ],
        }
    }

    #[test]
    fn test_open_creates_tables() {
        let db = test_db();
        let books = db.load_books().unwrap();
        assert!(books.is_empty());
    }

    #[test]
    fn test_migrate_embedding_model_clears_on_change() {
        let db = test_db();
        // Seed embeddings as if produced by an older model.
        db.upsert_chapter_chunks(
            "ch1",
            Some("book1"),
            &[(0, "text", &[1.0, 2.0])],
            Some("h1"),
        )
        .unwrap();
        db.upsert_book_embedding("book1", Some(&[1.0, 2.0]), Some(&[1.0, 2.0]), None)
            .unwrap();
        assert_eq!(db.load_all_chunk_hits().unwrap().len(), 1);

        // Same model → no-op.
        db.migrate_embedding_model().unwrap();
        assert_eq!(db.load_all_chunk_hits().unwrap().len(), 1);

        // Different stored model → both embedding tables cleared + meta updated.
        db.conn
            .execute(
                "INSERT INTO meta (key, value) VALUES ('embedding_model', 'all-MiniLM-L6-v2')
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                [],
            )
            .unwrap();
        db.migrate_embedding_model().unwrap();
        assert!(db.load_all_chunk_hits().unwrap().is_empty());
        assert!(
            db.load_all_book_embeddings_with_titles()
                .unwrap()
                .is_empty()
        );
        let stored: String = db
            .conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'embedding_model'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored, crate::embeddings::MODEL_VERSION);
    }

    #[test]
    fn test_open_path_writes_to_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.db");
        let db = ServerDb::open_path(&path).unwrap();
        db.upsert_book(&sample_book("url-1")).unwrap();
        drop(db);
        let db2 = ServerDb::open_path(&path).unwrap();
        assert_eq!(db2.load_books().unwrap().len(), 1);
    }

    #[test]
    fn test_upsert_and_load() {
        let db = test_db();
        db.upsert_book(&sample_book("url-1")).unwrap();

        let books = db.load_books().unwrap();
        assert_eq!(books.len(), 1);
        assert_eq!(books[0].url, "url-1");
        assert_eq!(books[0].title, "Book url-1");
        assert_eq!(books[0].sessions.len(), 1);
        assert_eq!(books[0].sessions[0].name, "Initial");
        assert_eq!(books[0].tags, vec!["tag1"]);
        assert_eq!(books[0].chapters.len(), 2);
    }

    #[test]
    fn test_upsert_multiple_books() {
        let db = test_db();
        db.upsert_book(&sample_book("a")).unwrap();
        db.upsert_book(&sample_book("b")).unwrap();
        assert_eq!(db.load_books().unwrap().len(), 2);
    }

    #[test]
    fn test_upsert_overwrites_existing() {
        let db = test_db();
        db.upsert_book(&sample_book("same-url")).unwrap();

        let mut updated = sample_book("same-url");
        updated.title = "Updated".into();
        db.upsert_book(&updated).unwrap();

        let books = db.load_books().unwrap();
        assert_eq!(books.len(), 1);
        assert_eq!(books[0].title, "Updated");
    }

    #[test]
    fn test_create_session() {
        let db = test_db();
        db.upsert_book(&sample_book("url")).unwrap();
        let session = db.create_session("url", "Re-read", 10).unwrap();
        assert_eq!(session.name, "Re-read");
        assert_eq!(session.progress.current, 0);
    }

    #[test]
    fn test_update_session_progress() {
        let db = test_db();
        db.upsert_book(&sample_book("url")).unwrap();
        let session = db.create_session("url", "Test", 10).unwrap();
        db.update_session_progress(session.id, 5).unwrap();
        let books = db.load_books().unwrap();
        let s = books[0]
            .sessions
            .iter()
            .find(|s| s.id == session.id)
            .unwrap();
        assert_eq!(s.progress.current, 5);
    }

    #[test]
    fn test_rename_session() {
        let db = test_db();
        db.upsert_book(&sample_book("url")).unwrap();
        let session = db.create_session("url", "Old", 10).unwrap();
        assert_eq!(db.rename_session(session.id, "New").unwrap(), 1);
        let books = db.load_books().unwrap();
        let s = books[0]
            .sessions
            .iter()
            .find(|s| s.id == session.id)
            .unwrap();
        assert_eq!(s.name, "New");
    }

    #[test]
    fn test_rename_missing_session_affects_zero_rows() {
        let db = test_db();
        assert_eq!(db.rename_session(999999, "New").unwrap(), 0);
    }

    #[test]
    fn test_delete_session() {
        let db = test_db();
        db.upsert_book(&sample_book("url")).unwrap();
        let books = db.load_books().unwrap();
        assert_eq!(db.delete_session(books[0].sessions[0].id).unwrap(), 1);
        let s1 = db.create_session("url", "S1", 10).unwrap();
        let s2 = db.create_session("url", "S2", 10).unwrap();
        assert_eq!(db.delete_session(s1.id).unwrap(), 1);
        let books = db.load_books().unwrap();
        assert_eq!(books[0].sessions.len(), 1);
        assert_eq!(books[0].sessions[0].id, s2.id);
    }

    #[test]
    fn test_delete_missing_session_affects_zero_rows() {
        let db = test_db();
        assert_eq!(db.delete_session(999999).unwrap(), 0);
    }

    #[test]
    fn test_delete_session_clears_active_session_id() {
        let db = test_db();
        db.upsert_book(&sample_book("url")).unwrap();
        let session = db.create_session("url", "Active", 10).unwrap();
        db.set_active_session("url", Some(session.id)).unwrap();
        let books = db.load_books().unwrap();
        assert_eq!(books[0].active_session_id, Some(session.id));

        db.delete_session(session.id).unwrap();
        let books = db.load_books().unwrap();
        assert_eq!(books[0].active_session_id, None);
    }

    #[test]
    fn test_upsert_preserves_user_data_on_rescrape() {
        let db = test_db();
        let mut book = sample_book("url");
        book.status = BookStatus::Completed;
        book.tags = vec!["keep-tag".into()];
        db.upsert_book(&book).unwrap();

        let session = db.create_session("url", "Active", 10).unwrap();
        db.set_active_session("url", Some(session.id)).unwrap();

        // Fresh scraped book: status Reading, no tags, no active session.
        let mut scraped = sample_book("url");
        scraped.status = BookStatus::Reading;
        scraped.tags = vec![];
        scraped.active_session_id = None;
        db.upsert_book(&scraped).unwrap();

        let books = db.load_books().unwrap();
        assert_eq!(books[0].status, BookStatus::Completed);
        assert_eq!(books[0].tags, vec!["keep-tag"]);
        assert_eq!(books[0].active_session_id, Some(session.id));
    }

    #[test]
    fn test_update_status() {
        let db = test_db();
        db.upsert_book(&sample_book("url")).unwrap();
        db.update_status("url", &BookStatus::Dropped).unwrap();

        let books = db.load_books().unwrap();
        assert_eq!(books[0].status, BookStatus::Dropped);
    }

    #[test]
    fn test_delete_book() {
        let db = test_db();
        db.upsert_book(&sample_book("url")).unwrap();
        db.delete_book("url").unwrap();
        assert!(db.load_books().unwrap().is_empty());
    }

    #[test]
    fn test_delete_book_cascades_to_tags_and_chapters() {
        let db = test_db();
        db.upsert_book(&sample_book("url")).unwrap();
        db.delete_book("url").unwrap();

        db.upsert_book(&sample_book("url")).unwrap();
        let books = db.load_books().unwrap();
        assert_eq!(books[0].tags.len(), 1);
        assert_eq!(books[0].chapters.len(), 2);
    }

    #[test]
    fn test_delete_book_removes_embeddings() {
        let db = test_db();
        db.upsert_book(&sample_book("book1")).unwrap();
        db.upsert_chapter_chunks("ch1", Some("book1"), &[(0, "text", &[1.0, 2.0])], None)
            .unwrap();
        db.upsert_chapter_chunks("ch2", Some("book1"), &[(0, "text", &[3.0, 4.0])], None)
            .unwrap();
        db.upsert_book_embedding(
            "book1",
            Some(&[1.0, 2.0]),
            Some(&[3.0, 4.0]),
            Some(&["Fantasy".to_string()]),
        )
        .unwrap();

        db.delete_book("book1").unwrap();

        assert!(db.chapter_chunk_state("ch1").unwrap().0 == 0);
        assert!(db.chapter_chunk_state("ch2").unwrap().0 == 0);
        assert!(db.get_book_embedding("book1").unwrap().is_none());
        assert!(db.load_all_chunk_hits().unwrap().is_empty());
        assert!(db.load_all_book_embeddings().unwrap().is_empty());
    }

    #[test]
    fn test_delete_book_leaves_other_books_embeddings_untouched() {
        let db = test_db();
        db.upsert_book(&sample_book("book1")).unwrap();
        db.upsert_book(&sample_book("book2")).unwrap();
        db.upsert_chapter_chunks("ch1", Some("book1"), &[(0, "text", &[1.0, 2.0])], None)
            .unwrap();
        db.upsert_chapter_chunks("ch2", Some("book2"), &[(0, "text", &[3.0, 4.0])], None)
            .unwrap();
        db.upsert_book_embedding("book1", None, Some(&[1.0, 2.0]), None)
            .unwrap();
        db.upsert_book_embedding("book2", None, Some(&[3.0, 4.0]), None)
            .unwrap();

        db.delete_book("book1").unwrap();

        // book1's embeddings are gone.
        assert!(db.chapter_chunk_state("ch1").unwrap().0 == 0);
        assert!(db.get_book_embedding("book1").unwrap().is_none());
        // book2's embeddings are untouched.
        assert!(db.chapter_chunk_state("ch2").unwrap().0 == 1);
        assert!(db.get_book_embedding("book2").unwrap().is_some());
    }

    #[test]
    fn test_sync_tags_replaces() {
        let db = test_db();
        db.upsert_book(&sample_book("url")).unwrap();

        let mut book = sample_book("url");
        book.tags = vec!["new-tag".into()];
        db.upsert_book(&book).unwrap();

        let books = db.load_books().unwrap();
        assert_eq!(books[0].tags, vec!["new-tag"]);
    }

    #[test]
    fn test_sync_chapters_replaces() {
        let db = test_db();
        db.upsert_book(&sample_book("url")).unwrap();

        let mut book = sample_book("url");
        book.chapters = vec![Chapter {
            url: "ch-new".into(),
            title: "New".into(),
            order: 99,
        }];
        db.upsert_book(&book).unwrap();

        let books = db.load_books().unwrap();
        assert_eq!(books[0].chapters.len(), 1);
        assert_eq!(books[0].chapters[0].title, "New");
    }

    #[test]
    fn test_status_str_roundtrip() {
        for status in &[
            BookStatus::Reading,
            BookStatus::Paused,
            BookStatus::Dropped,
            BookStatus::Completed,
        ] {
            let s = status_str(status);
            assert_eq!(&parse_status(s), status);
        }
    }

    #[test]
    fn test_parse_status_unknown_defaults_to_reading() {
        assert_eq!(parse_status("garbage"), BookStatus::Reading);
        assert_eq!(parse_status(""), BookStatus::Reading);
    }

    #[test]
    fn test_upsert_book_with_empty_chapters() {
        let db = test_db();
        let mut book = sample_book("url");
        book.chapters = vec![];
        db.upsert_book(&book).unwrap();

        let books = db.load_books().unwrap();
        assert!(books[0].chapters.is_empty());
    }

    #[test]
    fn test_upsert_book_with_empty_tags() {
        let db = test_db();
        let mut book = sample_book("url");
        book.tags = vec![];
        db.upsert_book(&book).unwrap();

        let books = db.load_books().unwrap();
        assert!(books[0].tags.is_empty());
    }

    #[test]
    fn test_migrate_from_old_schema() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE books (
                url         TEXT PRIMARY KEY,
                title       TEXT NOT NULL,
                status      TEXT NOT NULL DEFAULT 'Reading',
                current     INTEGER NOT NULL DEFAULT 0,
                total       INTEGER NOT NULL DEFAULT 0,
                cover_url   TEXT,
                description TEXT
            );
            INSERT INTO books (url, title, current, total) VALUES ('url-1', 'Old Book', 3, 10);
            INSERT INTO books (url, title, current, total) VALUES ('url-2', 'New Book', 0, 5);

            CREATE TABLE tags (
                book_url TEXT NOT NULL REFERENCES books(url) ON DELETE CASCADE,
                tag      TEXT NOT NULL,
                PRIMARY KEY (book_url, tag)
            );

            CREATE TABLE chapters (
                book_url TEXT NOT NULL REFERENCES books(url) ON DELETE CASCADE,
                url      TEXT NOT NULL,
                title    TEXT NOT NULL,
                ord      INTEGER NOT NULL,
                PRIMARY KEY (book_url, url)
            );",
        )
        .unwrap();

        let db = ServerDb::open_conn(conn).unwrap();
        let books = db.load_books().unwrap();
        assert_eq!(books.len(), 2);

        for book in &books {
            assert!(
                !book.sessions.is_empty(),
                "Book {} has no sessions",
                book.url
            );
            assert_eq!(book.sessions[0].name, "Initial");
            assert!(
                book.active_session_id.is_some(),
                "Book {} has no active session",
                book.url
            );
        }

        let old = books.iter().find(|b| b.url == "url-1").unwrap();
        assert_eq!(old.sessions[0].progress.current, 3);
        assert_eq!(old.sessions[0].progress.total, 10);

        let new = books.iter().find(|b| b.url == "url-2").unwrap();
        assert_eq!(new.sessions[0].progress.current, 0);
        assert_eq!(new.sessions[0].progress.total, 5);
    }

    #[test]
    fn test_load_books_ordered_by_rowid() {
        let db = test_db();
        db.upsert_book(&sample_book("b")).unwrap();
        db.upsert_book(&sample_book("a")).unwrap();
        db.upsert_book(&sample_book("c")).unwrap();

        let books = db.load_books().unwrap();
        assert_eq!(books[0].url, "b");
        assert_eq!(books[1].url, "a");
        assert_eq!(books[2].url, "c");
    }

    #[test]
    fn test_upsert_chapter_chunks_and_state() {
        let db = test_db();
        let emb = vec![0.1f32, 0.2, 0.3];
        db.upsert_chapter_chunks("ch1", Some("book1"), &[(0, "text", &emb)], Some("hash1"))
            .unwrap();

        let (count, hash) = db.chapter_chunk_state("ch1").unwrap();
        assert_eq!(count, 1);
        assert_eq!(hash.as_deref(), Some("hash1"));
    }

    #[test]
    fn test_upsert_chapter_chunks_replaces_atomically() {
        let db = test_db();
        db.upsert_chapter_chunks("ch1", Some("book1"), &[(0, "a", &[1.0, 2.0])], Some("h1"))
            .unwrap();
        // Re-embed with two chunks — the old single chunk must be replaced.
        db.upsert_chapter_chunks(
            "ch1",
            Some("book2"),
            &[(0, "a", &[3.0, 4.0]), (1, "b", &[5.0, 6.0])],
            Some("h2"),
        )
        .unwrap();

        let (count, hash) = db.chapter_chunk_state("ch1").unwrap();
        assert_eq!(count, 2);
        assert_eq!(hash.as_deref(), Some("h2"));
        let hits = db.load_all_chunk_hits().unwrap();
        assert_eq!(hits.len(), 2);
        assert!(hits.iter().all(|h| h.book_url == "book2"));
    }

    #[test]
    fn test_chapter_chunk_state_missing() {
        let db = test_db();
        let (count, hash) = db.chapter_chunk_state("nope").unwrap();
        assert_eq!(count, 0);
        assert!(hash.is_none());
    }

    #[test]
    fn test_upsert_and_get_book_embedding() {
        let db = test_db();
        let desc = vec![1.0f32, 2.0];
        let agg = vec![3.0f32, 4.0];
        let genres = vec!["Fantasy".to_string(), "LitRPG".to_string()];
        db.upsert_book_embedding("book1", Some(&desc), Some(&agg), Some(&genres))
            .unwrap();

        let (got_desc, got_agg, got_genres) = db.get_book_embedding("book1").unwrap().unwrap();
        assert_eq!(got_desc, Some(desc));
        assert_eq!(got_agg, Some(agg));
        assert_eq!(got_genres, Some(genres));
    }

    #[test]
    fn test_upsert_book_embedding_preserves_aggregate_when_none() {
        let db = test_db();
        let desc = vec![1.0f32, 2.0];
        let agg = vec![3.0f32, 4.0];
        db.upsert_book_embedding("book1", Some(&desc), Some(&agg), None)
            .unwrap();

        // Update only the description; aggregate must be preserved.
        let new_desc = vec![5.0f32, 6.0];
        db.upsert_book_embedding("book1", Some(&new_desc), None, None)
            .unwrap();

        let (got_desc, got_agg, _) = db.get_book_embedding("book1").unwrap().unwrap();
        assert_eq!(got_desc, Some(new_desc));
        assert_eq!(got_agg, Some(agg));
    }

    #[test]
    fn test_get_missing_book_embedding_returns_none() {
        let db = test_db();
        assert!(db.get_book_embedding("nope").unwrap().is_none());
    }

    #[test]
    fn test_load_chapter_embeddings_for_book() {
        let db = test_db();
        db.upsert_chapter_chunks("ch1", Some("book1"), &[(0, "text", &[1.0, 2.0])], None)
            .unwrap();
        db.upsert_chapter_chunks("ch2", Some("book1"), &[(0, "text", &[3.0, 4.0])], None)
            .unwrap();
        db.upsert_chapter_chunks("other", Some("book2"), &[(0, "text", &[9.0, 9.0])], None)
            .unwrap();

        let embs = db.load_chapter_embeddings_for_book("book1").unwrap();
        assert_eq!(embs, vec![vec![1.0, 2.0], vec![3.0, 4.0]]);
    }

    #[test]
    fn test_load_chapter_embeddings_for_book_averages_chunks() {
        let db = test_db();
        // ch1 has two chunks; the aggregate gets one averaged vector per chapter.
        db.upsert_chapter_chunks(
            "ch1",
            Some("book1"),
            &[(0, "a", &[1.0, 0.0]), (1, "b", &[3.0, 0.0])],
            None,
        )
        .unwrap();

        let embs = db.load_chapter_embeddings_for_book("book1").unwrap();
        assert_eq!(embs, vec![vec![2.0, 0.0]]);
    }

    #[test]
    fn test_find_book_url_for_chapter() {
        let db = test_db();
        db.upsert_book(&sample_book("book1")).unwrap();

        assert_eq!(
            db.find_book_url_for_chapter("ch1").unwrap(),
            Some("book1".to_string())
        );
        assert_eq!(db.find_book_url_for_chapter("missing").unwrap(), None);
    }

    #[test]
    fn test_f32_blob_roundtrip() {
        let v = vec![0.0f32, -1.5, 3.25, f32::MAX, f32::MIN];
        assert_eq!(decode_f32s(&encode_f32s(&v)), v);
    }

    #[test]
    fn test_books_missing_description_embedding() {
        let db = test_db();
        db.upsert_book(&sample_book("book1")).unwrap();
        db.upsert_book(&sample_book("book2")).unwrap();

        // book1 gets a description embedding; book2 stays missing.
        db.upsert_book_embedding("book1", Some(&[1.0, 2.0]), None, None)
            .unwrap();

        let missing = db.books_missing_description_embedding().unwrap();
        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].0, "book2");
        assert_eq!(missing[0].1, "desc");
    }

    #[test]
    fn test_books_missing_description_embedding_skips_null_descriptions() {
        let db = test_db();
        let mut book = sample_book("book1");
        book.description = None;
        db.upsert_book(&book).unwrap();

        assert!(db.books_missing_description_embedding().unwrap().is_empty());
    }

    #[test]
    fn test_load_all_chunk_hits() {
        let db = test_db();
        // Seed a book with chapters + genres so the join enriches the rows.
        db.upsert_book(&sample_book("book1")).unwrap();
        db.upsert_book_embedding(
            "book1",
            None,
            Some(&[1.0, 1.0]),
            Some(&["Fantasy".to_string()]),
        )
        .unwrap();
        db.upsert_chapter_chunks("ch1", Some("book1"), &[(0, "text", &[1.0, 2.0])], None)
            .unwrap();
        db.upsert_chapter_chunks("ch2", Some("book1"), &[(0, "text", &[3.0, 4.0])], None)
            .unwrap();
        db.upsert_chapter_chunks("orphan", None, &[(0, "text", &[9.0, 9.0])], None)
            .unwrap();

        let all = db.load_all_chunk_hits().unwrap();
        assert_eq!(all.len(), 3);
        assert!(all.contains(&crate::embeddings::ChunkHit {
            book_url: "book1".to_string(),
            book_title: "Book book1".to_string(),
            chapter_url: "ch1".to_string(),
            chapter_title: "Chapter 1".to_string(),
            chapter_idx: 1,
            genres: Some(vec!["Fantasy".to_string()]),
            chunk_idx: 0,
            text: "text".to_string(),
            embedding: vec![1.0, 2.0],
        }));
        assert!(all.contains(&crate::embeddings::ChunkHit {
            book_url: "book1".to_string(),
            book_title: "Book book1".to_string(),
            chapter_url: "ch2".to_string(),
            chapter_title: "Chapter 2".to_string(),
            chapter_idx: 2,
            genres: Some(vec!["Fantasy".to_string()]),
            chunk_idx: 0,
            text: "text".to_string(),
            embedding: vec![3.0, 4.0],
        }));
        // Orphan chapter: no book/chapter/genre enrichment.
        assert!(all.contains(&crate::embeddings::ChunkHit {
            book_url: String::new(),
            book_title: String::new(),
            chapter_url: "orphan".to_string(),
            chapter_title: String::new(),
            chapter_idx: 0,
            genres: None,
            chunk_idx: 0,
            text: "text".to_string(),
            embedding: vec![9.0, 9.0],
        }));
    }

    #[test]
    fn test_load_chapters_public() {
        let db = test_db();
        db.upsert_book(&sample_book("book1")).unwrap();
        let chapters = db.load_chapters("book1").unwrap();
        assert_eq!(chapters.len(), 2);
        assert_eq!(chapters[0].url, "ch1");
        assert_eq!(chapters[0].order, 1);
    }

    #[test]
    fn test_load_all_book_embeddings() {
        let db = test_db();
        db.upsert_book_embedding(
            "book1",
            Some(&[1.0, 2.0]),
            Some(&[3.0, 4.0]),
            Some(&["Fantasy".to_string()]),
        )
        .unwrap();
        db.upsert_book_embedding("book2", Some(&[5.0, 6.0]), Some(&[7.0, 8.0]), None)
            .unwrap();
        // book3 has no aggregate -> excluded.
        db.upsert_book_embedding("book3", Some(&[9.0, 9.0]), None, None)
            .unwrap();

        let all = db.load_all_book_embeddings().unwrap();
        assert_eq!(all.len(), 2);
        assert!(all.contains(&(
            "book1".to_string(),
            vec![3.0, 4.0],
            Some(vec!["Fantasy".to_string()])
        )));
        assert!(all.contains(&("book2".to_string(), vec![7.0, 8.0], None)));
    }

    #[test]
    fn test_embedding_status_full() {
        let db = test_db();
        db.upsert_book(&sample_book("book1")).unwrap();
        db.upsert_chapter_chunks("ch1", Some("book1"), &[(0, "text", &[1.0, 2.0])], None)
            .unwrap();
        db.upsert_chapter_chunks("ch2", Some("book1"), &[(0, "text", &[3.0, 4.0])], None)
            .unwrap();
        db.upsert_book_embedding(
            "book1",
            None,
            Some(&[3.0, 4.0]),
            Some(&["Fantasy".to_string(), "LitRPG".to_string()]),
        )
        .unwrap();

        let (embedded, total, has_agg, genres, urls) =
            db.embedding_status("book1").unwrap().unwrap();
        assert_eq!(embedded, 2);
        assert_eq!(total, 2);
        assert!(has_agg);
        assert_eq!(
            genres,
            Some(vec!["Fantasy".to_string(), "LitRPG".to_string()])
        );
        assert_eq!(urls, vec!["ch1".to_string(), "ch2".to_string()]);
    }

    #[test]
    fn test_embedding_status_no_aggregate() {
        let db = test_db();
        db.upsert_book(&sample_book("book1")).unwrap();
        db.upsert_chapter_chunks("ch1", Some("book1"), &[(0, "text", &[1.0, 2.0])], None)
            .unwrap();
        // No book_embeddings row at all.
        let (embedded, total, has_agg, genres, urls) =
            db.embedding_status("book1").unwrap().unwrap();
        assert_eq!(embedded, 1);
        assert_eq!(total, 2);
        assert!(!has_agg);
        assert_eq!(genres, None);
        assert_eq!(urls, vec!["ch1".to_string()]);
    }

    #[test]
    fn test_embedding_status_no_embedded_chapters() {
        let db = test_db();
        db.upsert_book(&sample_book("book1")).unwrap();
        // No chapter embeddings at all.
        let (embedded, total, has_agg, genres, urls) =
            db.embedding_status("book1").unwrap().unwrap();
        assert_eq!(embedded, 0);
        assert_eq!(total, 2);
        assert!(!has_agg);
        assert_eq!(genres, None);
        assert!(urls.is_empty());
    }

    #[test]
    fn test_embedding_status_missing_book_returns_none() {
        let db = test_db();
        assert!(db.embedding_status("nope").unwrap().is_none());
    }
}
