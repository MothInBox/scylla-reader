//! Types for channel communication between the main thread and worker.

use crate::models::{Book, Job, JobId, JobKind, JobOutcome, JobPriority, JobStatus};

pub enum AppCommand {
    Scrape(String),
    UpdateAll(Vec<String>),
    FetchChapter(String, usize),
    SetRateLimit(u64),
    FetchCover(String),
    // new job commands
    Enqueue(JobKind, String, JobPriority),  // kind, target display, priority
    CancelJob(JobId),
    CancelAll,
    RetryJob(JobId),
    RetryAllFailed,
    FlushCompleted,
    FlushAll,
    SetMaxWorkers(u8),
    ReorderJob(JobId, usize),
    InstallPlugin(String), // GitHub repo URL
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
    // new job events
    JobEnqueued(Job),
    JobStatusChanged(JobId, JobStatus),
    WorkersChanged(u8),
    JobOutcome(JobId, JobOutcome),
    PluginInstalled(String, String),   // domain, wasm path
    PluginInstallFailed(String),       // error message
}

#[cfg(test)]
mod tests {
    use super::*;
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

    #[test]
    fn test_app_command_enqueue_construction() {
        let cmd = AppCommand::Enqueue(
            JobKind::Scrape("http://example.com".into()),
            "Example".into(),
            JobPriority::High,
        );
        if let AppCommand::Enqueue(kind, target, priority) = &cmd {
            assert_eq!(kind.target(), "http://example.com");
            assert_eq!(target, "Example");
            assert_eq!(*priority, JobPriority::High);
        } else {
            panic!("Expected Enqueue variant");
        }
    }

    #[test]
    fn test_app_command_cancel_job_construction() {
        let cmd = AppCommand::CancelJob(42);
        if let AppCommand::CancelJob(id) = &cmd {
            assert_eq!(*id, 42);
        } else {
            panic!("Expected CancelJob variant");
        }
    }

    #[test]
    fn test_app_command_set_max_workers_construction() {
        let cmd = AppCommand::SetMaxWorkers(8);
        if let AppCommand::SetMaxWorkers(n) = &cmd {
            assert_eq!(*n, 8);
        } else {
            panic!("Expected SetMaxWorkers variant");
        }
    }

    #[test]
    fn test_app_event_job_enqueued_construction() {
        let job = Job::new(
            1,
            JobKind::Scrape("http://example.com".into()),
            "Example".into(),
            JobPriority::Normal,
        );
        let event = AppEvent::JobEnqueued(job);
        if let AppEvent::JobEnqueued(j) = &event {
            assert_eq!(j.id, 1);
        } else {
            panic!("Expected JobEnqueued variant");
        }
    }

    #[test]
    fn test_app_command_install_plugin_construction() {
        let cmd = AppCommand::InstallPlugin("https://github.com/owner/repo".into());
        if let AppCommand::InstallPlugin(url) = &cmd {
            assert_eq!(url, "https://github.com/owner/repo");
        } else {
            panic!("Expected InstallPlugin variant");
        }
    }

    #[test]
    fn test_app_event_plugin_installed_construction() {
        let event = AppEvent::PluginInstalled("example.com".into(), "/path/to/plugin.wasm".into());
        if let AppEvent::PluginInstalled(domain, path) = &event {
            assert_eq!(domain, "example.com");
            assert_eq!(path, "/path/to/plugin.wasm");
        } else {
            panic!("Expected PluginInstalled variant");
        }
    }

    #[test]
    fn test_app_event_plugin_install_failed_construction() {
        let event = AppEvent::PluginInstallFailed("Network error".into());
        if let AppEvent::PluginInstallFailed(msg) = &event {
            assert_eq!(msg, "Network error");
        } else {
            panic!("Expected PluginInstallFailed variant");
        }
    }
}
