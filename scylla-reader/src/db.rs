use rusqlite::{Connection, Result, params};
use crate::models::{Book, BookStatus, Chapter, Progress};

pub struct Db {
    conn: Connection,
}

impl Db {
    pub fn open() -> Result<Self> {
        let path = data_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let conn = Connection::open(&path)?;
        let db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    fn migrate(&self) -> Result<()> {
        self.conn.execute_batch("
            CREATE TABLE IF NOT EXISTS books (
                url         TEXT PRIMARY KEY,
                title       TEXT NOT NULL,
                status      TEXT NOT NULL DEFAULT 'Reading',
                current     INTEGER NOT NULL DEFAULT 0,
                total       INTEGER NOT NULL DEFAULT 0,
                cover_url   TEXT,
                description TEXT
            );

            CREATE TABLE IF NOT EXISTS tags (
                book_url    TEXT NOT NULL REFERENCES books(url) ON DELETE CASCADE,
                tag         TEXT NOT NULL,
                PRIMARY KEY (book_url, tag)
            );

            CREATE TABLE IF NOT EXISTS chapters (
                book_url    TEXT NOT NULL REFERENCES books(url) ON DELETE CASCADE,
                url         TEXT NOT NULL,
                title       TEXT NOT NULL,
                ord         INTEGER NOT NULL,
                PRIMARY KEY (book_url, url)
            );

            PRAGMA foreign_keys = ON;
        ")
    }

    // ── Load ────────────────────────────────────────────────────────────────

    pub fn load_books(&self) -> Result<Vec<Book>> {
        let mut stmt = self.conn.prepare(
            "SELECT url, title, status, current, total, cover_url, description FROM books ORDER BY rowid"
        )?;

        let mut books: Vec<Book> = stmt.query_map([], |row| {
            let status_str: String = row.get(2)?;
            Ok(Book {
                url:         row.get(0)?,
                title:       row.get(1)?,
                status:      parse_status(&status_str),
                progress:    Progress { current: row.get(3)?, total: row.get(4)? },
                cover_url:   row.get(5)?,
                description: row.get(6)?,
                tags:        Vec::new(),
                chapters:    Vec::new(),
            })
        })?.collect::<Result<_>>()?;

        for book in &mut books {
            book.tags     = self.load_tags(&book.url)?;
            book.chapters = self.load_chapters(&book.url)?;
        }

        Ok(books)
    }

    fn load_tags(&self, book_url: &str) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT tag FROM tags WHERE book_url = ? ORDER BY tag"
        )?;
        stmt.query_map([book_url], |row| row.get(0))?.collect()
    }

    fn load_chapters(&self, book_url: &str) -> Result<Vec<Chapter>> {
        let mut stmt = self.conn.prepare(
            "SELECT url, title, ord FROM chapters WHERE book_url = ? ORDER BY ord"
        )?;
        stmt.query_map([book_url], |row| {
            Ok(Chapter {
                url:   row.get(0)?,
                title: row.get(1)?,
                order: row.get(2)?,
            })
        })?.collect()
    }

    // ── Upsert book (title, meta, progress) ────────────────────────────────

    pub fn upsert_book(&self, book: &Book) -> Result<()> {
        self.conn.execute(
            "INSERT INTO books (url, title, status, current, total, cover_url, description)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(url) DO UPDATE SET
               title       = excluded.title,
               status      = excluded.status,
               current     = excluded.current,
               total       = excluded.total,
               cover_url   = excluded.cover_url,
               description = excluded.description",
            params![
                book.url,
                book.title,
                status_str(&book.status),
                book.progress.current,
                book.progress.total,
                book.cover_url,
                book.description,
            ],
        )?;

        self.sync_tags(&book.url, &book.tags)?;
        self.sync_chapters(&book.url, &book.chapters)?;
        Ok(())
    }

    // ── Fast field updates (called on every status/progress change) ─────────

    pub fn update_progress(&self, book_url: &str, current: u32, total: u32) -> Result<()> {
        self.conn.execute(
            "UPDATE books SET current = ?1, total = ?2 WHERE url = ?3",
            params![current, total, book_url],
        )?;
        Ok(())
    }

    pub fn update_status(&self, book_url: &str, status: &BookStatus) -> Result<()> {
        self.conn.execute(
            "UPDATE books SET status = ?1 WHERE url = ?2",
            params![status_str(status), book_url],
        )?;
        Ok(())
    }

    // ── Delete ──────────────────────────────────────────────────────────────

    pub fn delete_book(&self, book_url: &str) -> Result<()> {
        // child rows cascade via FK
        self.conn.execute("DELETE FROM books WHERE url = ?", [book_url])?;
        Ok(())
    }

    // ── Internal sync helpers ───────────────────────────────────────────────

    fn sync_tags(&self, book_url: &str, tags: &[String]) -> Result<()> {
        self.conn.execute("DELETE FROM tags WHERE book_url = ?", [book_url])?;
        let mut stmt = self.conn.prepare(
            "INSERT OR IGNORE INTO tags (book_url, tag) VALUES (?1, ?2)"
        )?;
        for tag in tags {
            stmt.execute(params![book_url, tag])?;
        }
        Ok(())
    }

    fn sync_chapters(&self, book_url: &str, chapters: &[Chapter]) -> Result<()> {
        self.conn.execute("DELETE FROM chapters WHERE book_url = ?", [book_url])?;
        let mut stmt = self.conn.prepare(
            "INSERT OR IGNORE INTO chapters (book_url, url, title, ord) VALUES (?1, ?2, ?3, ?4)"
        )?;
        for ch in chapters {
            stmt.execute(params![book_url, ch.url, ch.title, ch.order])?;
        }
        Ok(())
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn data_path() -> std::path::PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("scylla-reader")
        .join("library.db")
}

fn status_str(s: &BookStatus) -> &'static str {
    match s {
        BookStatus::Reading   => "Reading",
        BookStatus::Paused    => "Paused",
        BookStatus::Dropped   => "Dropped",
        BookStatus::Completed => "Completed",
    }
}

fn parse_status(s: &str) -> BookStatus {
    match s {
        "Paused"    => BookStatus::Paused,
        "Dropped"   => BookStatus::Dropped,
        "Completed" => BookStatus::Completed,
        _           => BookStatus::Reading,
    }
}
