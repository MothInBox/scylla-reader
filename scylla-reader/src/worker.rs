//! Background worker thread that runs a Tokio runtime, owns the ScraperRegistry,
//! and processes AppCommand messages, sending results back as AppEvents.

use crate::messenger::{AppCommand, AppEvent, ChapterContent};
use crate::scrapers::services::ScraperRegistry;
use std::sync::mpsc;

pub struct Worker {
    cmd_rx: mpsc::Receiver<AppCommand>,
    event_tx: mpsc::Sender<AppEvent>,
    registry: ScraperRegistry,
    rate_limit_secs: u64,
}

impl Worker {
    pub fn new(
        cmd_rx: mpsc::Receiver<AppCommand>,
        event_tx: mpsc::Sender<AppEvent>,
        registry: ScraperRegistry,
    ) -> Self {
        Self {
            cmd_rx,
            event_tx,
            registry,
            rate_limit_secs: 2,
        }
    }

    pub fn run(mut self) {
        let runtime = tokio::runtime::Runtime::new().expect("Failed to create Tokio runtime");

        while let Ok(command) = self.cmd_rx.recv() {
            match command {
                AppCommand::Scrape(url) => {
                    Self::scrape_and_send(
                        &runtime,
                        &self.registry,
                        &self.event_tx,
                        &clean_url(&url),
                    );
                }
                AppCommand::UpdateAll(urls) => {
                    let mut urls_iter = urls.into_iter();
                    if let Some(url) = urls_iter.next() {
                        Self::scrape_and_send(
                            &runtime,
                            &self.registry,
                            &self.event_tx,
                            &clean_url(&url),
                        );
                    }
                    'urls: for url in urls_iter {
                        let deadline = std::time::Instant::now()
                            + std::time::Duration::from_secs(self.rate_limit_secs);
                        while std::time::Instant::now() < deadline {
                            match self
                                .cmd_rx
                                .recv_timeout(std::time::Duration::from_millis(100))
                            {
                                Ok(cmd) => match cmd {
                                    AppCommand::FetchChapter(url, idx) => {
                                        let url = clean_url(&url);
                                        match runtime.block_on(self.registry.scrape_chapter(&url)) {
                                            Ok((title, content)) => {
                                                let _ = self.event_tx.send(
                                                    AppEvent::ChapterFetched(ChapterContent {
                                                        chapter_idx: idx,
                                                        title,
                                                        content,
                                                    }),
                                                );
                                            }
                                            Err(e) => crate::settings::log(
                                                crate::settings::LogLevel::Debug,
                                                "SCRAPE",
                                                &format!("Chapter fetch failed: {}", e),
                                            ),
                                        }
                                    }
                                    AppCommand::Scrape(url) => {
                                        Self::scrape_and_send(
                                            &runtime,
                                            &self.registry,
                                            &self.event_tx,
                                            &clean_url(&url),
                                        );
                                    }
                                    AppCommand::SetRateLimit(secs) => {
                                        self.rate_limit_secs = secs;
                                    }
                                    AppCommand::UpdateAll(_) => {}
                                    AppCommand::FetchCover(_) => {}
                                },
                                Err(mpsc::RecvTimeoutError::Timeout) => {}
                                Err(mpsc::RecvTimeoutError::Disconnected) => break 'urls,
                            }
                        }
                        Self::scrape_and_send(
                            &runtime,
                            &self.registry,
                            &self.event_tx,
                            &clean_url(&url),
                        );
                    }
                }
                AppCommand::SetRateLimit(secs) => {
                    self.rate_limit_secs = secs;
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
                        Err(e) => crate::settings::log(
                            crate::settings::LogLevel::Debug,
                            "SCRAPE",
                            &format!("Chapter fetch failed: {}", e),
                        ),
                    }
                }
                AppCommand::FetchCover(url) => {
                    let event_tx = self.event_tx.clone();
                    std::thread::spawn(move || {
                        let mut picker = ratatui_image::picker::Picker::from_query_stdio()
                            .unwrap_or_else(|_| {
                                ratatui_image::picker::Picker::from_fontsize((8, 12))
                            });
                        match reqwest::blocking::get(&url) {
                            Ok(resp) => match resp.bytes() {
                                Ok(bytes) => match image::load_from_memory(&bytes) {
                                    Ok(img) => {
                                        let protocol = picker.new_resize_protocol(img);
                                        let _ =
                                            event_tx.send(AppEvent::CoverFetched(url, protocol));
                                    }
                                    Err(e) => crate::settings::log(
                                        crate::settings::LogLevel::Debug,
                                        "UI",
                                        &format!("Image decode: {}", e),
                                    ),
                                },
                                Err(e) => crate::settings::log(
                                    crate::settings::LogLevel::Debug,
                                    "UI",
                                    &format!("Cover bytes: {}", e),
                                ),
                            },
                            Err(e) => crate::settings::log(
                                crate::settings::LogLevel::Debug,
                                "UI",
                                &format!("Cover fetch: {}", e),
                            ),
                        }
                    });
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
                crate::settings::log(
                    crate::settings::LogLevel::Debug,
                    "SCRAPE",
                    &format!("Scraped: {}", book.title),
                );
                let _ = event_tx.send(AppEvent::BookScraped(book));
            }
            Err(e) => crate::settings::log(
                crate::settings::LogLevel::Debug,
                "SCRAPE",
                &format!("Scrape failed: {}", e),
            ),
        }
    }
}

fn clean_url(url: &str) -> String {
    if let Some(open) = url.find("](") {
        let after = open + 2;
        let mut depth = 0i32;
        for (i, ch) in url[after..].char_indices() {
            match ch {
                '(' => depth += 1,
                ')' => {
                    if depth == 0 {
                        return url[after..after + i].trim().to_string();
                    }
                    depth -= 1;
                }
                _ => {}
            }
        }
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

    #[test]
    fn test_clean_url_trailing_parens_text() {
        let input = "[link](https://url.com) and (stuff)";
        assert_eq!(clean_url(input), "https://url.com");
    }
}
