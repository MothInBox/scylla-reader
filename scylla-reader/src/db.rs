//! SQLite persistence layer — books, sessions, tags, and chapters.

use crate::models::{Book, BookStatus, Chapter, Progress, Session};
use rusqlite::{Connection, Result, params};

pub struct Db {
    conn: Connection,
}

// ── Construction ──────────────────────────────────────────────────────────────

impl Db {
    pub fn open() -> Result<Self> {
        let path = data_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        Self::open_conn(Connection::open(&path)?)
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

// ── Schema migrations ─────────────────────────────────────────────────────────

impl Db {
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
        ",
        )?;

        // v1: active_session_id column (old schema may not have it)
        let _ = self
            .conn
            .execute_batch("ALTER TABLE books ADD COLUMN active_session_id INTEGER;");

        // v2: migrate progress from old books table into sessions
        let _ = self.conn.execute(
            "INSERT INTO sessions (book_url, name, current, total)
             SELECT url, 'Initial', current, total FROM books
             WHERE NOT EXISTS (SELECT 1 FROM sessions WHERE sessions.book_url = books.url)",
            [],
        );

        // v3: ensure every book has an active session
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
}

// ── Reads ─────────────────────────────────────────────────────────────────────

impl Db {
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

    fn load_chapters(&self, book_url: &str) -> Result<Vec<Chapter>> {
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

// ── Writes ────────────────────────────────────────────────────────────────────

impl Db {
    /// Upsert a book and all its tags and chapters atomically.
    pub fn upsert_book(&self, book: &Book) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;

        tx.execute(
            "INSERT INTO books (url, title, status, active_session_id, cover_url, description)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(url) DO UPDATE SET
               title             = excluded.title,
               status            = excluded.status,
               active_session_id = excluded.active_session_id,
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

        // Tags
        tx.execute("DELETE FROM tags WHERE book_url = ?", [&book.url])?;
        {
            let mut stmt =
                tx.prepare("INSERT OR IGNORE INTO tags (book_url, tag) VALUES (?1, ?2)")?;
            for tag in &book.tags {
                stmt.execute(params![book.url, tag])?;
            }
        }

        // Chapters
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

        // Ensure at least one session exists (outside the tx — reads need it committed)
        self.ensure_default_session(&book.url, book.chapters.len() as u32)?;
        Ok(())
    }

    pub fn delete_book(&self, book_url: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM books WHERE url = ?", [book_url])?;
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

// ── Sessions ──────────────────────────────────────────────────────────────────

impl Db {
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

    pub fn rename_session(&self, id: i64, name: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE sessions SET name = ?1, updated_at = datetime('now') WHERE id = ?2",
            params![name, id],
        )?;
        Ok(())
    }

    pub fn delete_session(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM sessions WHERE id = ?", [id])?;
        Ok(())
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

// ── Helpers ───────────────────────────────────────────────────────────────────

fn data_path() -> std::path::PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("scylla-reader")
        .join("library.db")
}

fn status_str(s: &BookStatus) -> &'static str {
    match s {
        BookStatus::Reading => "Reading",
        BookStatus::Paused => "Paused",
        BookStatus::Dropped => "Dropped",
        BookStatus::Completed => "Completed",
    }
}

fn parse_status(s: &str) -> BookStatus {
    match s {
        "Paused" => BookStatus::Paused,
        "Dropped" => BookStatus::Dropped,
        "Completed" => BookStatus::Completed,
        _ => BookStatus::Reading,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> Db {
        Db::open_conn(Connection::open_in_memory().unwrap()).unwrap()
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
        assert!(test_db().load_books().unwrap().is_empty());
    }

    #[test]
    fn test_open_path_writes_to_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.db");
        let db = Db::open_path(&path).unwrap();
        db.upsert_book(&sample_book("url-1")).unwrap();
        drop(db);
        assert_eq!(Db::open_path(&path).unwrap().load_books().unwrap().len(), 1);
    }

    #[test]
    fn test_upsert_and_load() {
        let db = test_db();
        db.upsert_book(&sample_book("url-1")).unwrap();
        let books = db.load_books().unwrap();
        assert_eq!(books.len(), 1);
        assert_eq!(books[0].url, "url-1");
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
        let s = db.create_session("url", "Re-read", 10).unwrap();
        assert_eq!(s.name, "Re-read");
        assert_eq!(s.progress.current, 0);
    }

    #[test]
    fn test_update_session_progress() {
        let db = test_db();
        db.upsert_book(&sample_book("url")).unwrap();
        let s = db.create_session("url", "Test", 10).unwrap();
        db.update_session_progress(s.id, 5).unwrap();
        let books = db.load_books().unwrap();
        let found = books[0].sessions.iter().find(|x| x.id == s.id).unwrap();
        assert_eq!(found.progress.current, 5);
    }

    #[test]
    fn test_rename_session() {
        let db = test_db();
        db.upsert_book(&sample_book("url")).unwrap();
        let s = db.create_session("url", "Old", 10).unwrap();
        db.rename_session(s.id, "New").unwrap();
        let books = db.load_books().unwrap();
        let found = books[0].sessions.iter().find(|x| x.id == s.id).unwrap();
        assert_eq!(found.name, "New");
    }

    #[test]
    fn test_delete_session() {
        let db = test_db();
        db.upsert_book(&sample_book("url")).unwrap();
        let books = db.load_books().unwrap();
        db.delete_session(books[0].sessions[0].id).unwrap();
        let s1 = db.create_session("url", "S1", 10).unwrap();
        let s2 = db.create_session("url", "S2", 10).unwrap();
        db.delete_session(s1.id).unwrap();
        let books = db.load_books().unwrap();
        assert_eq!(books[0].sessions.len(), 1);
        assert_eq!(books[0].sessions[0].id, s2.id);
    }

    #[test]
    fn test_update_status() {
        let db = test_db();
        db.upsert_book(&sample_book("url")).unwrap();
        db.update_status("url", &BookStatus::Dropped).unwrap();
        assert_eq!(db.load_books().unwrap()[0].status, BookStatus::Dropped);
    }

    #[test]
    fn test_delete_book() {
        let db = test_db();
        db.upsert_book(&sample_book("url")).unwrap();
        db.delete_book("url").unwrap();
        assert!(db.load_books().unwrap().is_empty());
    }

    #[test]
    fn test_delete_book_cascades() {
        let db = test_db();
        db.upsert_book(&sample_book("url")).unwrap();
        db.delete_book("url").unwrap();
        db.upsert_book(&sample_book("url")).unwrap();
        let books = db.load_books().unwrap();
        assert_eq!(books[0].tags.len(), 1);
        assert_eq!(books[0].chapters.len(), 2);
    }

    #[test]
    fn test_sync_tags_replaces() {
        let db = test_db();
        db.upsert_book(&sample_book("url")).unwrap();
        let mut book = sample_book("url");
        book.tags = vec!["new-tag".into()];
        db.upsert_book(&book).unwrap();
        assert_eq!(db.load_books().unwrap()[0].tags, vec!["new-tag"]);
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
    fn test_status_roundtrip() {
        for status in &[
            BookStatus::Reading,
            BookStatus::Paused,
            BookStatus::Dropped,
            BookStatus::Completed,
        ] {
            assert_eq!(&parse_status(status_str(status)), status);
        }
    }

    #[test]
    fn test_parse_status_unknown_defaults_to_reading() {
        assert_eq!(parse_status("garbage"), BookStatus::Reading);
        assert_eq!(parse_status(""), BookStatus::Reading);
    }

    #[test]
    fn test_data_path_ends_with_db() {
        assert!(data_path().to_string_lossy().ends_with("library.db"));
    }

    #[test]
    fn test_upsert_book_with_empty_chapters() {
        let db = test_db();
        let mut book = sample_book("url");
        book.chapters = vec![];
        db.upsert_book(&book).unwrap();
        assert!(db.load_books().unwrap()[0].chapters.is_empty());
    }

    #[test]
    fn test_upsert_book_with_empty_tags() {
        let db = test_db();
        let mut book = sample_book("url");
        book.tags = vec![];
        db.upsert_book(&book).unwrap();
        assert!(db.load_books().unwrap()[0].tags.is_empty());
    }

    #[test]
    fn test_migrate_from_old_schema() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "
            CREATE TABLE books (
                url TEXT PRIMARY KEY, title TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'Reading',
                current INTEGER NOT NULL DEFAULT 0,
                total INTEGER NOT NULL DEFAULT 0,
                cover_url TEXT, description TEXT
            );
            INSERT INTO books (url, title, current, total) VALUES ('url-1', 'Old Book', 3, 10);
            INSERT INTO books (url, title, current, total) VALUES ('url-2', 'New Book', 0, 5);
            CREATE TABLE tags (
                book_url TEXT NOT NULL REFERENCES books(url) ON DELETE CASCADE,
                tag TEXT NOT NULL, PRIMARY KEY (book_url, tag)
            );
            CREATE TABLE chapters (
                book_url TEXT NOT NULL REFERENCES books(url) ON DELETE CASCADE,
                url TEXT NOT NULL, title TEXT NOT NULL, ord INTEGER NOT NULL,
                PRIMARY KEY (book_url, url)
            );
        ",
        )
        .unwrap();

        let db = Db::open_conn(conn).unwrap();
        let books = db.load_books().unwrap();
        assert_eq!(books.len(), 2);
        for book in &books {
            assert!(!book.sessions.is_empty());
            assert_eq!(book.sessions[0].name, "Initial");
            assert!(book.active_session_id.is_some());
        }
        let old = books.iter().find(|b| b.url == "url-1").unwrap();
        assert_eq!(old.sessions[0].progress.current, 3);
        assert_eq!(old.sessions[0].progress.total, 10);
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
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> Db {
        let conn = Connection::open_in_memory().unwrap();
        Db::open_conn(conn).unwrap()
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
    fn test_open_path_writes_to_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.db");
        let db = Db::open_path(&path).unwrap();
        db.upsert_book(&sample_book("url-1")).unwrap();
        drop(db);
        let db2 = Db::open_path(&path).unwrap();
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
        db.rename_session(session.id, "New").unwrap();
        let books = db.load_books().unwrap();
        let s = books[0]
            .sessions
            .iter()
            .find(|s| s.id == session.id)
            .unwrap();
        assert_eq!(s.name, "New");
    }

    #[test]
    fn test_delete_session() {
        let db = test_db();
        db.upsert_book(&sample_book("url")).unwrap();
        // upsert_book created a default "Initial" session; remove it.
        let books = db.load_books().unwrap();
        db.delete_session(books[0].sessions[0].id).unwrap();
        let s1 = db.create_session("url", "S1", 10).unwrap();
        let s2 = db.create_session("url", "S2", 10).unwrap();
        db.delete_session(s1.id).unwrap();
        let books = db.load_books().unwrap();
        assert_eq!(books[0].sessions.len(), 1);
        assert_eq!(books[0].sessions[0].id, s2.id);
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
    fn test_data_path_contains_db_filename() {
        let path = data_path();
        assert!(path.to_string_lossy().ends_with("library.db"));
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
        // Simulate opening a database created with the old schema (has current/total cols)
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

        let db = Db::open_conn(conn).unwrap();
        let books = db.load_books().unwrap();
        assert_eq!(books.len(), 2);

        // Both books should have migrated sessions
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

        // Old book should have preserved current=3
        let old = books.iter().find(|b| b.url == "url-1").unwrap();
        assert_eq!(old.sessions[0].progress.current, 3);
        assert_eq!(old.sessions[0].progress.total, 10);

        // New book should have current=0, total=5
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
}
