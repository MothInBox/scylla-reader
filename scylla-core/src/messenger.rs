use crate::types::{Book, ChapterDetail, ChapterRef, Job, JobId, JobOutcome, JobStatus};

pub enum AppCommand {
    Scrape(String),
    UpdateAll(Vec<String>),
    FetchChapter(String, usize),
    SetRateLimit(u64),
    FetchCover(String),
    EmbedBatch(Vec<ChapterRef>),
    CancelJob(JobId),
    CancelAll,
    RetryJob(JobId),
    RetryAllFailed,
    FlushCompleted,
    FlushAll,
    SetMaxWorkers(u8),
    InstallPlugin(String),
}

pub struct ChapterContent {
    pub url: String,
    pub chapter_idx: usize,
    pub title: String,
    pub content: String,
}

pub enum AppEvent {
    BookScraped(Book),
    ChapterFetched(ChapterContent),
    /// A chapter that must be embedded regardless of the autoembed setting
    /// (e.g. from an EmbedBatch job). Carries the originating job id.
    ChapterToEmbed(JobId, ChapterContent),
    /// A chapter embedding was stored (job id, chapter url).
    ChapterEmbedded(JobId, String),
    /// A chapter embedding failed (job id, chapter url).
    ChapterEmbeddedFailed(JobId, String),
    ChapterFetchFailed,
    /// (url, raw image bytes) — the TUI decodes the image.
    CoverFetched(String, Vec<u8>),
    JobEnqueued(Job),
    JobStatusChanged(JobId, JobStatus),
    JobDetailChanged(JobId, Vec<ChapterDetail>),
    WorkersChanged(u8),
    JobOutcome(JobId, JobOutcome),
    PluginInstalled(String, String),
    PluginInstallFailed(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::BookStatus;

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
            url: "http://example.com/ch3".into(),
            chapter_idx: 3,
            title: "Chapter 3".into(),
            content: "Some content".into(),
        };
        let event = AppEvent::ChapterFetched(content);
        if let AppEvent::ChapterFetched(c) = &event {
            assert_eq!(c.url, "http://example.com/ch3");
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
        let event = AppEvent::CoverFetched("http://example.com/cover.jpg".into(), vec![1, 2, 3]);
        if let AppEvent::CoverFetched(url, bytes) = &event {
            assert_eq!(url, "http://example.com/cover.jpg");
            assert_eq!(bytes, &vec![1, 2, 3]);
        } else {
            panic!("Expected CoverFetched variant");
        }
    }

    #[test]
    fn test_chapter_content_field_access() {
        let content = ChapterContent {
            url: "http://example.com/ch1".into(),
            chapter_idx: 1,
            title: "Chapter Title".into(),
            content: "Chapter Content".into(),
        };
        assert_eq!(content.url, "http://example.com/ch1");
        assert_eq!(content.chapter_idx, 1);
        assert_eq!(content.title, "Chapter Title");
        assert_eq!(content.content, "Chapter Content");
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
            crate::types::JobKind::Scrape("http://example.com".into()),
            "Example".into(),
            crate::types::JobPriority::Normal,
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

    #[test]
    fn test_app_command_embed_batch_construction() {
        let chapters = vec![ChapterRef {
            url: "http://example.com/ch1".into(),
            idx: 0,
            title: "Ch1".into(),
        }];
        let cmd = AppCommand::EmbedBatch(chapters);
        if let AppCommand::EmbedBatch(chs) = &cmd {
            assert_eq!(chs.len(), 1);
            assert_eq!(chs[0].url, "http://example.com/ch1");
            assert_eq!(chs[0].idx, 0);
        } else {
            panic!("Expected EmbedBatch variant");
        }
    }

    #[test]
    fn test_app_event_job_detail_changed_construction() {
        let detail = vec![ChapterDetail {
            title: "Ch1".into(),
            url: "http://example.com/ch1".into(),
            status: "Done".into(),
        }];
        let event = AppEvent::JobDetailChanged(7, detail);
        if let AppEvent::JobDetailChanged(id, d) = &event {
            assert_eq!(*id, 7);
            assert_eq!(d[0].status, "Done");
        } else {
            panic!("Expected JobDetailChanged variant");
        }
    }

    #[test]
    fn test_app_event_chapter_to_embed_construction() {
        let content = ChapterContent {
            url: "http://example.com/ch1".into(),
            chapter_idx: 0,
            title: "Ch1".into(),
            content: "text".into(),
        };
        let event = AppEvent::ChapterToEmbed(7, content);
        if let AppEvent::ChapterToEmbed(id, c) = &event {
            assert_eq!(*id, 7);
            assert_eq!(c.url, "http://example.com/ch1");
            assert_eq!(c.chapter_idx, 0);
        } else {
            panic!("Expected ChapterToEmbed variant");
        }
    }

    #[test]
    fn test_app_event_chapter_embedded_construction() {
        let event = AppEvent::ChapterEmbedded(7, "http://example.com/ch1".into());
        if let AppEvent::ChapterEmbedded(id, url) = &event {
            assert_eq!(*id, 7);
            assert_eq!(url, "http://example.com/ch1");
        } else {
            panic!("Expected ChapterEmbedded variant");
        }
    }

    #[test]
    fn test_app_event_chapter_embedded_failed_construction() {
        let event = AppEvent::ChapterEmbeddedFailed(7, "http://example.com/ch1".into());
        if let AppEvent::ChapterEmbeddedFailed(id, url) = &event {
            assert_eq!(*id, 7);
            assert_eq!(url, "http://example.com/ch1");
        } else {
            panic!("Expected ChapterEmbeddedFailed variant");
        }
    }
}
