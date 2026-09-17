use crate::messenger::{AppCommand, AppEvent, ChapterContent};
use crate::scraper::ScraperRegistry;
use crate::types::{ChapterDetail, Job, JobId, JobKind, JobOutcome, JobPriority, JobStatus};
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
}

impl JobManager {
    pub fn new(
        cmd_rx: mpsc::Receiver<AppCommand>,
        event_tx: mpsc::Sender<AppEvent>,
        registry: ScraperRegistry,
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
                        let cleaned = clean_url(url);
                        let reg = registry.lock().unwrap();
                        match runtime_ref.block_on(reg.scrape_chapter(&cleaned)) {
                            Ok((title, content)) => {
                                let _ = event_tx.send(AppEvent::ChapterFetched(ChapterContent {
                                    url: url.clone(),
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
                    JobKind::FetchCover(url) => match reqwest::blocking::get(url) {
                        Ok(resp) => match resp.bytes() {
                            Ok(bytes) => {
                                let _ = event_tx
                                    .send(AppEvent::CoverFetched(url.clone(), bytes.to_vec()));
                                let o = JobOutcome::CoverFetched;
                                let _ = event_tx.send(AppEvent::JobOutcome(job.id, o.clone()));
                                (Ok(()), Some(o))
                            }
                            Err(e) => (Err(format!("Cover bytes: {}", e)), None),
                        },
                        Err(e) => (Err(format!("Cover fetch: {}", e)), None),
                    },
                    JobKind::EmbedBatch(chapters) => {
                        let mut detail: Vec<ChapterDetail> = chapters
                            .iter()
                            .map(|c| ChapterDetail {
                                title: c.title.clone(),
                                url: c.url.clone(),
                                status: "Pending".into(),
                            })
                            .collect();
                        let mut any_ok = false;
                        let mut last_err = String::new();
                        for (i, ch) in chapters.iter().enumerate() {
                            let reg = registry.lock().unwrap();
                            match runtime_ref.block_on(reg.scrape_chapter(&ch.url)) {
                                Ok((title, content)) => {
                                    let _ = event_tx.send(AppEvent::ChapterToEmbed(
                                        job.id,
                                        ChapterContent {
                                            url: ch.url.clone(),
                                            chapter_idx: ch.idx,
                                            title: title.clone(),
                                            content: content.clone(),
                                        },
                                    ));
                                    detail[i].status = "Fetched".into();
                                    any_ok = true;
                                }
                                Err(e) => {
                                    detail[i].status = "Failed".into();
                                    last_err = e.to_string();
                                }
                            }
                            let _ =
                                event_tx.send(AppEvent::JobDetailChanged(job.id, detail.clone()));
                        }
                        if any_ok {
                            (Ok(()), None)
                        } else {
                            (Err(last_err), None)
                        }
                    }
                };

                if outcome.is_some() {
                    crate::log::log(
                        "DEBUG",
                        "WORKER",
                        &format!("Job {} completed with outcome", id),
                    );
                }
                match result {
                    Ok(_) => {
                        // EmbedBatch jobs stay Running after the fetches — the
                        // server marks them Completed once every chapter is
                        // embedded (or failed to embed).
                        let status = if matches!(job.kind, JobKind::EmbedBatch(_)) {
                            JobStatus::Running
                        } else {
                            JobStatus::Completed
                        };
                        let _ = event_tx.send(AppEvent::JobStatusChanged(id, status));
                    }
                    Err(e) => {
                        crate::log::log("DEBUG", "WORKER", &format!("Job {} failed: {}", id, e));
                        let _ = event_tx.send(AppEvent::JobStatusChanged(id, JobStatus::Failed(e)));
                    }
                }
            });

            self.active_jobs.insert(id, handle);
        }
    }

    fn handle_command(&mut self, cmd: AppCommand) {
        match cmd {
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
                    let _ = self
                        .event_tx
                        .send(AppEvent::JobStatusChanged(id, JobStatus::Queued));
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
                        let _ = self
                            .event_tx
                            .send(AppEvent::JobStatusChanged(id, JobStatus::Queued));
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
                let id = self.next_job_id;
                self.next_job_id += 1;
                let target = format!("{} ch{}", url, idx);
                let job = Job::new(
                    id,
                    JobKind::FetchChapter(url, idx),
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
            AppCommand::EmbedBatch(chapters) => {
                let id = self.next_job_id;
                self.next_job_id += 1;
                let target = format!("Embed {} chapters", chapters.len());
                let mut job =
                    Job::new(id, JobKind::EmbedBatch(chapters), target, JobPriority::High);
                job.detail = Some(
                    job.kind
                        .chapters()
                        .map(|chs| {
                            chs.iter()
                                .map(|c| ChapterDetail {
                                    title: c.title.clone(),
                                    url: c.url.clone(),
                                    status: "Pending".into(),
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                );
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
                crate::log::log(
                    "DEBUG",
                    "PLUGIN",
                    &format!("Installing plugin from: {}", repo_url),
                );
                match crate::plugin_install::install_plugin(&repo_url) {
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
    use crate::types::ChapterRef;

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

    fn test_manager() -> (JobManager, std::sync::mpsc::Receiver<AppEvent>) {
        let (_cmd_tx, cmd_rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();
        let manager = JobManager::new(cmd_rx, event_tx, ScraperRegistry::new(), 4, 2);
        (manager, event_rx)
    }

    #[test]
    fn test_retry_job_emits_queued_event() {
        let (mut manager, event_rx) = test_manager();
        manager.handle_command(AppCommand::Scrape("http://example.com".into()));
        let id = manager.job_queue[0].id;
        manager.job_queue[0].status = JobStatus::Failed("boom".into());

        manager.handle_command(AppCommand::RetryJob(id));

        // First event is the JobEnqueued from the Scrape command.
        let _ = event_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        let event = event_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(matches!(
            event,
            AppEvent::JobStatusChanged(jid, JobStatus::Queued) if jid == id
        ));
        // Original failed job + re-queued retry.
        assert_eq!(manager.job_queue.len(), 2);
        assert_eq!(manager.job_queue[1].status, JobStatus::Queued);
        assert_eq!(manager.job_queue[1].error, None);
    }

    #[test]
    fn test_retry_all_failed_emits_queued_events() {
        let (mut manager, event_rx) = test_manager();
        manager.handle_command(AppCommand::Scrape("http://example.com/a".into()));
        manager.handle_command(AppCommand::Scrape("http://example.com/b".into()));
        let ids: Vec<JobId> = manager.job_queue.iter().map(|j| j.id).collect();
        for job in manager.job_queue.iter_mut() {
            job.status = JobStatus::Failed("boom".into());
        }

        manager.handle_command(AppCommand::RetryAllFailed);

        // Two JobEnqueued events from the Scrape commands.
        let _ = event_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        let _ = event_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        let e1 = event_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        let e2 = event_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(matches!(
            e1,
            AppEvent::JobStatusChanged(jid, JobStatus::Queued) if ids.contains(&jid)
        ));
        assert!(matches!(
            e2,
            AppEvent::JobStatusChanged(jid, JobStatus::Queued) if ids.contains(&jid)
        ));
        // Original failed jobs + two re-queued retries.
        assert_eq!(manager.job_queue.len(), 4);
        assert!(
            manager
                .job_queue
                .iter()
                .filter(|j| j.status == JobStatus::Queued)
                .count()
                == 2
        );
    }

    #[test]
    fn test_embed_batch_enqueue_sets_detail() {
        let (mut manager, event_rx) = test_manager();
        let chapters = vec![
            ChapterRef {
                url: "http://example.com/ch1".into(),
                idx: 0,
                title: "Ch1".into(),
            },
            ChapterRef {
                url: "http://example.com/ch2".into(),
                idx: 1,
                title: "Ch2".into(),
            },
        ];
        manager.handle_command(AppCommand::EmbedBatch(chapters));

        let event = event_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(matches!(event, AppEvent::JobEnqueued(_)));
        assert_eq!(manager.job_queue.len(), 1);
        let job = &manager.job_queue[0];
        assert!(matches!(job.kind, JobKind::EmbedBatch(_)));
        let detail = job.detail.as_ref().unwrap();
        assert_eq!(detail.len(), 2);
        assert_eq!(detail[0].status, "Pending");
        assert_eq!(detail[0].title, "Ch1");
        assert_eq!(detail[1].url, "http://example.com/ch2");
    }
}
