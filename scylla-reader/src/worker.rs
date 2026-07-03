//! Background worker thread that runs a Tokio runtime, owns the ScraperRegistry,
//! and processes AppCommand messages, sending results back as AppEvents.

use std::sync::mpsc;
use crate::messenger::{AppCommand, AppEvent, ChapterContent};
use crate::scrapers::services::ScraperRegistry;

pub struct Worker {
    cmd_rx: mpsc::Receiver<AppCommand>,
    event_tx: mpsc::Sender<AppEvent>,
    registry: ScraperRegistry,
}

impl Worker {
    pub fn new(
        cmd_rx: mpsc::Receiver<AppCommand>,
        event_tx: mpsc::Sender<AppEvent>,
        registry: ScraperRegistry,
    ) -> Self {
        Self { cmd_rx, event_tx, registry }
    }

    pub fn run(self) {
        let runtime = tokio::runtime::Runtime::new().expect("Failed to create Tokio runtime");

        while let Ok(command) = self.cmd_rx.recv() {
            match command {
                AppCommand::Scrape(url) => {
                    Self::scrape_and_send(&runtime, &self.registry, &self.event_tx, &clean_url(&url));
                }
                AppCommand::UpdateAll(urls) => {
                    for url in urls {
                        Self::scrape_and_send(&runtime, &self.registry, &self.event_tx, &clean_url(&url));
                        std::thread::sleep(std::time::Duration::from_secs(2));
                    }
                }
AppCommand::FetchChapter(url, idx) => {
    let url = clean_url(&url);
    match runtime.block_on(self.registry.scrape_chapter(&url)) {
        Ok((title, content)) => {
            let _ = self.event_tx.send(AppEvent::ChapterFetched(ChapterContent {
                chapter_idx: idx,
                title,
                content,
            }));
        }
        Err(e) => crate::settings::log(crate::settings::LogLevel::Debug, "SCRAPE", &format!("Chapter fetch failed: {}", e)),
    }
}
            }
        }
    }

    fn scrape_and_send(
        runtime: &tokio::runtime::Runtime,
        registry: &ScraperRegistry,
        event_tx: &mpsc::Sender<AppEvent>,
        url: &str,
    ) {
        match runtime.block_on(registry.scrape_url(url)) {
            Ok(book) => {
                crate::settings::log(crate::settings::LogLevel::Debug, "SCRAPE", &format!("Scraped: {}", book.title));
                let _ = event_tx.send(AppEvent::BookScraped(book));
            }
            Err(e) => crate::settings::log(crate::settings::LogLevel::Debug, "SCRAPE", &format!("Scrape failed: {}", e)),
        }
    }
}

fn clean_url(url: &str) -> String {
    if let (Some(open), Some(close)) = (url.find("]("), url.rfind(')')) {
        return url[open + 2..close].trim().to_string();
    }
    url.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clean_url_no_brackets() {
        assert_eq!(clean_url("https://example.com"), "https://example.com");
    }

    #[test]
    fn test_clean_url_markdown() {
        let input = "[click here](https://example.com/page)";
        assert_eq!(clean_url(input), "https://example.com/page");
    }

    #[test]
    fn test_clean_url_markdown_with_trailing_text() {
        let input = "[text](https://example.com) and more";
        assert_eq!(clean_url(input), "https://example.com");
    }

    #[test]
    fn test_clean_url_trims_whitespace() {
        assert_eq!(clean_url("  https://example.com  "), "https://example.com");
    }

    #[test]
    fn test_clean_url_empty_input() {
        assert_eq!(clean_url(""), "");
    }

    #[test]
    fn test_clean_url_nested_parens() {
        let input = "[link](https://en.wikipedia.org/wiki/Rust_(programming_language))";
        assert_eq!(
            clean_url(input),
            "https://en.wikipedia.org/wiki/Rust_(programming_language)"
        );
    }

    #[test]
    fn test_clean_url_no_close_bracket() {
        let input = "[text](https://example.com";
        assert_eq!(clean_url(input), "[text](https://example.com");
    }
}
