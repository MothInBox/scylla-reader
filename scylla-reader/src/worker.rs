use std::sync::mpsc;
use crate::models::Book;
use crate::messenger::{AppCommand, ChapterContent};
use crate::scrapers::services::ScraperRegistry;

pub struct Worker {
    cmd_rx: mpsc::Receiver<AppCommand>,
    ui_tx: mpsc::Sender<Book>,
    chapter_tx: mpsc::Sender<ChapterContent>,
}

impl Worker {
    pub fn new(
        cmd_rx: mpsc::Receiver<AppCommand>,
        ui_tx: mpsc::Sender<Book>,
        chapter_tx: mpsc::Sender<ChapterContent>,
    ) -> Self {
        Self { cmd_rx, ui_tx, chapter_tx }
    }

    pub fn run(self) {
        let runtime = tokio::runtime::Runtime::new().expect("Failed to create Tokio runtime");
        let registry = ScraperRegistry::new();

        while let Ok(command) = self.cmd_rx.recv() {
            match command {
                AppCommand::Scrape(url) => {
                    Self::scrape_and_send(&runtime, &registry, &self.ui_tx, &clean_url(&url));
                }
                AppCommand::UpdateAll(urls) => {
                    for url in urls {
                        Self::scrape_and_send(&runtime, &registry, &self.ui_tx, &clean_url(&url));
                        std::thread::sleep(std::time::Duration::from_secs(2));
                    }
                }
AppCommand::FetchChapter(url, idx) => {
    let url = clean_url(&url);
    match runtime.block_on(registry.scrape_chapter(&url)) {
        Ok((title, content)) => {
            let _ = self.chapter_tx.send(ChapterContent {
                chapter_idx: idx,
                title,
                content,
            });
        }
        Err(e) => crate::settings::log_debug(&format!("Chapter fetch failed: {}", e)),
    }
}
            }
        }
    }

    fn scrape_and_send(
        runtime: &tokio::runtime::Runtime,
        registry: &ScraperRegistry,
        ui_tx: &mpsc::Sender<Book>,
        url: &str,
    ) {
        match runtime.block_on(registry.scrape_url(url)) {
            Ok(book) => {
                crate::settings::log_debug(&format!("Scraped: {}", book.title));
                let _ = ui_tx.send(book);
            }
            Err(e) => crate::settings::log_debug(&format!("Scrape failed: {}", e)),
        }
    }
}

fn clean_url(url: &str) -> String {
    // Strip markdown link format [text](url)
    if let (Some(open), Some(close)) = (url.find("]("), url.rfind(')')) {
        return url[open + 2..close].trim().to_string();
    }
    url.trim().to_string()
}
