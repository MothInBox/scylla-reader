//! SQLite persistence layer — books, tags, and chapters.

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

    pub fn delete_book(&self, book_url: &str) -> Result<()> {
        self.conn.execute("DELETE FROM books WHERE url = ?", [book_url])?;
        Ok(())
    }

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

    pub fn open_conn(conn: Connection) -> Result<Db> {
        let db = Db { conn };
        db.migrate()?;
        Ok(db)
    }

    pub fn open_path(path: &std::path::Path) -> Result<Db> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let conn = Connection::open(path)?;
        let db = Db { conn };
        db.migrate()?;
        Ok(db)
    }
}

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
            progress: Progress { current: 0, total: 10 },
            tags: vec!["tag1".into()],
            cover_url: None,
            description: Some("desc".into()),
            chapters: vec![
                Chapter { url: "ch1".into(), title: "Chapter 1".into(), order: 1 },
                Chapter { url: "ch2".into(), title: "Chapter 2".into(), order: 2 },
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
        assert_eq!(books[0].progress.total, 10);
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
        updated.progress.current = 5;
        db.upsert_book(&updated).unwrap();

        let books = db.load_books().unwrap();
        assert_eq!(books.len(), 1);
        assert_eq!(books[0].title, "Updated");
        assert_eq!(books[0].progress.current, 5);
    }

    #[test]
    fn test_update_progress() {
        let db = test_db();
        db.upsert_book(&sample_book("url")).unwrap();
        db.update_progress("url", 7, 10).unwrap();

        let books = db.load_books().unwrap();
        assert_eq!(books[0].progress.current, 7);
        assert_eq!(books[0].progress.total, 10);
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
        book.chapters = vec![Chapter { url: "ch-new".into(), title: "New".into(), order: 99 }];
        db.upsert_book(&book).unwrap();

        let books = db.load_books().unwrap();
        assert_eq!(books[0].chapters.len(), 1);
        assert_eq!(books[0].chapters[0].title, "New");
    }

    #[test]
    fn test_status_str_roundtrip() {
        for status in &[BookStatus::Reading, BookStatus::Paused, BookStatus::Dropped, BookStatus::Completed] {
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
