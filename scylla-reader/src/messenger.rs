//! Types for channel communication between the main thread and worker.

use crate::models::Book;

pub enum AppCommand {
    Scrape(String),
    UpdateAll(Vec<String>),
    FetchChapter(String, usize),
    SetRateLimit(u64),
    FetchCover(String),
}

pub struct ChapterContent {
    pub chapter_idx: usize,
    pub title: String,
    pub content: String,
}

pub enum AppEvent {
    BookScraped(Book),
    ChapterFetched(ChapterContent),
    ChapterFetchFailed,
    CoverFetched(String, ratatui_image::protocol::StatefulProtocol),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Book;
    use crate::models::BookStatus;

    #[test]
    fn test_app_command_scrape_construction() {
        let cmd = AppCommand::Scrape("http://example.com".into());
        if let AppCommand::Scrape(url) = &cmd {
            assert_eq!(url, "http://example.com");
        } else {
            panic!("Expected Scrape variant");
        }
    }

    #[test]
    fn test_app_command_update_all_construction() {
        let urls = vec!["url1".into(), "url2".into()];
        let cmd = AppCommand::UpdateAll(urls);
        if let AppCommand::UpdateAll(u) = &cmd {
            assert_eq!(u.len(), 2);
            assert_eq!(u[0], "url1");
            assert_eq!(u[1], "url2");
        } else {
            panic!("Expected UpdateAll variant");
        }
    }

    #[test]
    fn test_app_command_fetch_chapter_construction() {
        let cmd = AppCommand::FetchChapter("http://example.com/ch1".into(), 5);
        if let AppCommand::FetchChapter(url, idx) = &cmd {
            assert_eq!(url, "http://example.com/ch1");
            assert_eq!(*idx, 5);
        } else {
            panic!("Expected FetchChapter variant");
        }
    }

    #[test]
    fn test_app_command_set_rate_limit_construction() {
        let cmd = AppCommand::SetRateLimit(1000);
        if let AppCommand::SetRateLimit(ms) = &cmd {
            assert_eq!(*ms, 1000);
        } else {
            panic!("Expected SetRateLimit variant");
        }
    }

    #[test]
    fn test_app_command_fetch_cover_construction() {
        let cmd = AppCommand::FetchCover("http://example.com/cover.jpg".into());
        if let AppCommand::FetchCover(url) = &cmd {
            assert_eq!(url, "http://example.com/cover.jpg");
        } else {
            panic!("Expected FetchCover variant");
        }
    }

    #[test]
    fn test_app_event_book_scraped_construction() {
        let book = Book {
            title: "Test Book".into(),
            url: "http://example.com".into(),
            status: BookStatus::Reading,
            sessions: vec![],
            active_session_id: None,
            tags: vec![],
            cover_url: None,
            description: None,
            chapters: vec![],
        };
        let event = AppEvent::BookScraped(book);
        if let AppEvent::BookScraped(b) = &event {
            assert_eq!(b.title, "Test Book");
            assert_eq!(b.url, "http://example.com");
        } else {
            panic!("Expected BookScraped variant");
        }
    }

    #[test]
    fn test_app_event_chapter_fetched_with_content() {
        let content = ChapterContent {
            chapter_idx: 3,
            title: "Chapter 3".into(),
            content: "Some content".into(),
        };
        let event = AppEvent::ChapterFetched(content);
        if let AppEvent::ChapterFetched(c) = &event {
            assert_eq!(c.chapter_idx, 3);
            assert_eq!(c.title, "Chapter 3");
            assert_eq!(c.content, "Some content");
        } else {
            panic!("Expected ChapterFetched variant");
        }
    }

    #[test]
    fn test_app_event_chapter_fetch_failed() {
        let event = AppEvent::ChapterFetchFailed;
        assert!(matches!(event, AppEvent::ChapterFetchFailed));
    }

    #[test]
    fn test_app_event_cover_fetched_construction() {
        let img = image::DynamicImage::new(1, 1, image::ColorType::Rgb8);
        let source = ratatui_image::protocol::ImageSource::new(img, (10, 10));
        let halfblocks = ratatui_image::protocol::halfblocks::StatefulHalfblocks::new(source, (10, 10));
        let protocol = ratatui_image::protocol::StatefulProtocol::Halfblocks(halfblocks);
        let event = AppEvent::CoverFetched("http://example.com/cover.jpg".into(), protocol);
        if let AppEvent::CoverFetched(url, _proto) = &event {
            assert_eq!(url, "http://example.com/cover.jpg");
        } else {
            panic!("Expected CoverFetched variant");
        }
    }

    #[test]
    fn test_chapter_content_field_access() {
        let content = ChapterContent {
            chapter_idx: 1,
            title: "Chapter Title".into(),
            content: "Chapter Content".into(),
        };
        assert_eq!(content.chapter_idx, 1);
        assert_eq!(content.title, "Chapter Title");
        assert_eq!(content.content, "Chapter Content");
    }
}
