//! Background worker thread that runs a Tokio runtime, owns the ScraperRegistry,
//! and processes AppCommand messages, sending results back as AppEvents.

use crate::messenger::{AppCommand, AppEvent, ChapterContent};
use crate::models::job::{Job, JobId, JobKind, JobOutcome, JobPriority, JobStatus};
use crate::scrapers::services::ScraperRegistry;
use std::collections::HashMap;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub struct JobManager {
    cmd_rx: mpsc::Receiver<AppCommand>,
    event_tx: mpsc::Sender<AppEvent>,
    registry: Arc<Mutex<ScraperRegistry>>,
    job_queue: Vec<Job>,
    delayed_queue: Vec<Job>,
    active_jobs: HashMap<JobId, tokio::task::JoinHandle<()>>,
    last_request: HashMap<String, Instant>,
    next_job_id: JobId,
    rate_limit_secs: u64,
    max_workers: u8,
    picker_font_size: (u16, u16),
    picker_protocol_type: ratatui_image::picker::ProtocolType,
}

impl JobManager {
    pub fn new(
        cmd_rx: mpsc::Receiver<AppCommand>,
        event_tx: mpsc::Sender<AppEvent>,
        registry: ScraperRegistry,
        picker_font_size: (u16, u16),
        picker_protocol_type: ratatui_image::picker::ProtocolType,
        max_workers: u8,
        rate_limit_secs: u64,
    ) -> Self {
        Self {
            cmd_rx,
            event_tx,
            registry: Arc::new(Mutex::new(registry)),
            job_queue: Vec::new(),
            delayed_queue: Vec::new(),
            active_jobs: HashMap::new(),
            last_request: HashMap::new(),
            next_job_id: 1,
            rate_limit_secs,
            max_workers,
            picker_font_size,
            picker_protocol_type,
        }
    }

    pub fn run(mut self) {
        let runtime = tokio::runtime::Runtime::new().expect("Failed to create Tokio runtime");

        loop {
            match self.cmd_rx.recv_timeout(Duration::from_millis(200)) {
                Ok(cmd) => self.handle_command(cmd),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }

            self.check_completed_jobs(&runtime);
            self.process_delayed_queue();
            self.try_spawn_jobs(&runtime);
        }
    }

    fn check_completed_jobs(&mut self, runtime: &tokio::runtime::Runtime) {
        let completed: Vec<JobId> = self
            .active_jobs
            .iter()
            .filter(|(_, h)| h.is_finished())
            .map(|(&id, _)| id)
            .collect();

        for id in completed {
            if let Some(handle) = self.active_jobs.remove(&id) {
                let _ = runtime.block_on(handle);
            }
        }
    }

    fn try_spawn_jobs(&mut self, runtime: &tokio::runtime::Runtime) {
        while self.active_jobs.len() < self.max_workers as usize && !self.job_queue.is_empty() {
            let domain = extract_domain(self.job_queue[0].kind.target());
            let cooled_down = match &domain {
                Some(d) => self
                    .last_request
                    .get(d)
                    .map(|last| last.elapsed() >= Duration::from_secs(self.rate_limit_secs))
                    .unwrap_or(true),
                None => true,
            };

            if !cooled_down {
                let job = self.job_queue.remove(0);
                self.delayed_queue.push(job);
                continue;
            }

            let mut job = self.job_queue.remove(0);
            let id = job.id;
            job.status = JobStatus::Running;
            let _ = self
                .event_tx
                .send(AppEvent::JobStatusChanged(id, JobStatus::Running));

            if let Some(d) = &domain {
                self.last_request.insert(d.clone(), Instant::now());
            }

            let event_tx = self.event_tx.clone();
            let registry = self.registry.clone();
            let runtime_ref = runtime.handle().clone();
            let font_size = self.picker_font_size;
            let protocol_type = self.picker_protocol_type;

            let handle = runtime.spawn_blocking(move || {
                let (result, outcome) = match &job.kind {
                    JobKind::Scrape(url) => {
                        let reg = registry.lock().unwrap();
                        match runtime_ref.block_on(reg.scrape_url(url)) {
                            Ok(book) => {
                                let _ = event_tx.send(AppEvent::BookScraped(book.clone()));
                                let o = JobOutcome::BookScraped {
                                    title: book.title,
                                    chapters: book.chapters.len(),
                                    cover: book.cover_url.is_some(),
                                };
                                let _ = event_tx.send(AppEvent::JobOutcome(job.id, o.clone()));
                                (Ok(()), Some(o))
                            }
                            Err(e) => (Err(e.to_string()), None),
                        }
                    }
                    JobKind::FetchChapter(url, idx) => {
                        let reg = registry.lock().unwrap();
                        match runtime_ref.block_on(reg.scrape_chapter(url)) {
                            Ok((title, content)) => {
                                let _ = event_tx.send(AppEvent::ChapterFetched(ChapterContent {
                                    chapter_idx: *idx,
                                    title: title.clone(),
                                    content: content.clone(),
                                }));
                                let o = JobOutcome::ChapterFetched {
                                    title,
                                    content_chars: content.len(),
                                };
                                let _ = event_tx.send(AppEvent::JobOutcome(job.id, o.clone()));
                                (Ok(()), Some(o))
                            }
                            Err(e) => (Err(e.to_string()), None),
                        }
                    }
                    JobKind::FetchCover(url) => {
                        let mut picker =
                            ratatui_image::picker::Picker::from_fontsize(font_size);
                        picker.set_protocol_type(protocol_type);
                        match reqwest::blocking::get(url) {
                            Ok(resp) => match resp.bytes() {
                                Ok(bytes) => match image::load_from_memory(&bytes) {
                                    Ok(img) => {
                                        let protocol = picker.new_resize_protocol(img);
                                        let _ = event_tx
                                            .send(AppEvent::CoverFetched(url.clone(), protocol));
                                        let o = JobOutcome::CoverFetched;
                                        let _ = event_tx.send(AppEvent::JobOutcome(job.id, o.clone()));
                                        (Ok(()), Some(o))
                                    }
                                    Err(e) => (Err(format!("Image decode: {}", e)), None),
                                },
                                Err(e) => (Err(format!("Cover bytes: {}", e)), None),
                            },
                            Err(e) => (Err(format!("Cover fetch: {}", e)), None),
                        }
                    }
                };

                if outcome.is_some() {
                    crate::settings::log(
                        crate::settings::LogLevel::Debug,
                        "WORKER",
                        &format!("Job {} completed with outcome", id),
                    );
                }
                match result {
                    Ok(_) => {
                        let _ = event_tx.send(AppEvent::JobStatusChanged(id, JobStatus::Completed));
                    }
                    Err(e) => {
                        crate::settings::log(
                            crate::settings::LogLevel::Debug,
                            "WORKER",
                            &format!("Job {} failed: {}", id, e),
                        );
                        let _ = event_tx.send(AppEvent::JobStatusChanged(id, JobStatus::Failed(e)));
                    }
                }
            });

            self.active_jobs.insert(id, handle);
        }
    }

    fn handle_command(&mut self, cmd: AppCommand) {
        match cmd {
            AppCommand::Enqueue(kind, target, priority) => {
                let id = self.next_job_id;
                self.next_job_id += 1;
                let job = Job::new(id, kind, target, priority);
                self.job_queue.push(job.clone());
                let _ = self.event_tx.send(AppEvent::JobEnqueued(job));
            }
            AppCommand::CancelJob(id) => {
                if let Some(handle) = self.active_jobs.remove(&id) {
                    handle.abort();
                }
                self.job_queue.retain(|j| j.id != id);
                self.delayed_queue.retain(|j| j.id != id);
                let _ = self
                    .event_tx
                    .send(AppEvent::JobStatusChanged(id, JobStatus::Cancelled));
            }
            AppCommand::CancelAll => {
                for (id, handle) in self.active_jobs.drain() {
                    handle.abort();
                    let _ = self
                        .event_tx
                        .send(AppEvent::JobStatusChanged(id, JobStatus::Cancelled));
                }
                for job in self.job_queue.drain(..) {
                    let _ = self
                        .event_tx
                        .send(AppEvent::JobStatusChanged(job.id, JobStatus::Cancelled));
                }
                for job in self.delayed_queue.drain(..) {
                    let _ = self
                        .event_tx
                        .send(AppEvent::JobStatusChanged(job.id, JobStatus::Cancelled));
                }
            }
            AppCommand::RetryJob(id) => {
                if let Some(job) = self.find_job_to_retry(id) {
                    let mut retry = job.clone();
                    retry.status = JobStatus::Queued;
                    retry.error = None;
                    retry.outcome = None;
                    retry.started_at = None;
                    retry.completed_at = None;
                    self.job_queue.push(retry);
                }
            }
            AppCommand::RetryAllFailed => {
                let all_jobs: Vec<&Job> = self
                    .job_queue
                    .iter()
                    .chain(self.delayed_queue.iter())
                    .collect();
                let failed_ids: Vec<JobId> = all_jobs
                    .iter()
                    .filter(|j| matches!(j.status, JobStatus::Failed(_)))
                    .map(|j| j.id)
                    .collect();
                for id in failed_ids {
                    if let Some(job) = self.find_job_to_retry(id) {
                        let mut retry = job.clone();
                        retry.status = JobStatus::Queued;
                        retry.error = None;
                        retry.started_at = None;
                        retry.completed_at = None;
                        self.job_queue.push(retry);
                    }
                }
            }
            AppCommand::FlushCompleted => {
                self.job_queue
                    .retain(|j| !matches!(j.status, JobStatus::Completed | JobStatus::Cancelled));
            }
            AppCommand::FlushAll => {
                for (id, handle) in self.active_jobs.drain() {
                    handle.abort();
                    let _ = self
                        .event_tx
                        .send(AppEvent::JobStatusChanged(id, JobStatus::Cancelled));
                }
                for job in self.job_queue.drain(..) {
                    let _ = self
                        .event_tx
                        .send(AppEvent::JobStatusChanged(job.id, JobStatus::Cancelled));
                }
                for job in self.delayed_queue.drain(..) {
                    let _ = self
                        .event_tx
                        .send(AppEvent::JobStatusChanged(job.id, JobStatus::Cancelled));
                }
            }
            AppCommand::SetMaxWorkers(n) => {
                self.max_workers = n;
                let _ = self.event_tx.send(AppEvent::WorkersChanged(n));
            }
            AppCommand::ReorderJob(id, new_pos) => {
                if let Some(pos) = self.job_queue.iter().position(|j| j.id == id) {
                    let job = self.job_queue.remove(pos);
                    let new_pos = new_pos.min(self.job_queue.len());
                    self.job_queue.insert(new_pos, job);
                }
            }
            // Legacy commands — wrap as Enqueue
            AppCommand::Scrape(url) => {
                let cleaned = clean_url(&url);
                let id = self.next_job_id;
                self.next_job_id += 1;
                let target = cleaned.clone();
                let job = Job::new(id, JobKind::Scrape(cleaned), target, JobPriority::High);
                self.job_queue.push(job.clone());
                let _ = self.event_tx.send(AppEvent::JobEnqueued(job));
            }
            AppCommand::FetchChapter(url, idx) => {
                let cleaned = clean_url(&url);
                let id = self.next_job_id;
                self.next_job_id += 1;
                let target = format!("{} ch{}", cleaned, idx);
                let job = Job::new(
                    id,
                    JobKind::FetchChapter(cleaned, idx),
                    target,
                    JobPriority::High,
                );
                self.job_queue.push(job.clone());
                let _ = self.event_tx.send(AppEvent::JobEnqueued(job));
            }
            AppCommand::FetchCover(url) => {
                let id = self.next_job_id;
                self.next_job_id += 1;
                let target = url.clone();
                let job = Job::new(id, JobKind::FetchCover(url), target, JobPriority::High);
                self.job_queue.push(job.clone());
                let _ = self.event_tx.send(AppEvent::JobEnqueued(job));
            }
            AppCommand::UpdateAll(urls) => {
                for url in urls {
                    let cleaned = clean_url(&url);
                    let id = self.next_job_id;
                    self.next_job_id += 1;
                    let target = cleaned.clone();
                    let job = Job::new(id, JobKind::Scrape(cleaned), target, JobPriority::Normal);
                    self.job_queue.push(job.clone());
                    let _ = self.event_tx.send(AppEvent::JobEnqueued(job));
                }
            }
            AppCommand::SetRateLimit(secs) => {
                self.rate_limit_secs = secs;
            }
            AppCommand::InstallPlugin(repo_url) => {
                crate::settings::log(
                    crate::settings::LogLevel::Debug,
                    "PLUGIN",
                    &format!("Installing plugin from: {}", repo_url),
                );
                match crate::scrapers::plugin_install::install_plugin(&repo_url) {
                    Ok((domain, path)) => {
                        let _ = self.event_tx.send(AppEvent::PluginInstalled(domain, path));
                    }
                    Err(msg) => {
                        let _ = self.event_tx.send(AppEvent::PluginInstallFailed(msg));
                    }
                }
            }
        }
    }

    fn find_job_to_retry(&self, id: JobId) -> Option<&Job> {
        self.job_queue
            .iter()
            .chain(self.delayed_queue.iter())
            .find(|j| j.id == id && matches!(j.status, JobStatus::Failed(_)))
    }

    fn process_delayed_queue(&mut self) {
        let now = Instant::now();
        let mut ready = Vec::new();
        self.delayed_queue.retain(|job| {
            let domain = extract_domain(job.kind.target());
            let cooled_down = match &domain {
                Some(d) => self
                    .last_request
                    .get(d)
                    .map(|last| {
                        now.duration_since(*last) >= Duration::from_secs(self.rate_limit_secs)
                    })
                    .unwrap_or(true),
                None => true,
            };
            if cooled_down {
                ready.push(job.clone());
                false
            } else {
                true
            }
        });
        for job in ready.into_iter().rev() {
            self.job_queue.insert(0, job);
        }
    }
}

fn extract_domain(url: &str) -> Option<String> {
    let url = url.trim();
    if url.is_empty() {
        return None;
    }
    let after_proto = if let Some(pos) = url.find("://") {
        &url[pos + 3..]
    } else {
        url
    };
    let host = after_proto.split('/').next().unwrap_or(after_proto);
    let host = host.split(':').next().unwrap_or(host);
    if host.is_empty() {
        None
    } else {
        Some(host.to_lowercase())
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

    #[test]
    fn test_extract_domain_simple() {
        assert_eq!(
            extract_domain("https://example.com/path"),
            Some("example.com".into())
        );
    }

    #[test]
    fn test_extract_domain_empty() {
        assert_eq!(extract_domain(""), None);
    }
}
