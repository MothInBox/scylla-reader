use scylla_core::types::{ChapterDetail, JobDto, JobFilter, JobOutcomeDto};
use std::collections::{HashMap, VecDeque};
use std::time::Instant;

/// Rolling completion-time windows for a job's fetch and embed phases.
#[derive(Debug, Default)]
pub struct JobTiming {
    /// Last N JobDetailChanged arrival times (fetch completions).
    pub fetch_times: VecDeque<Instant>,
    /// Last N ChapterEmbedded/ChapterEmbeddedFailed arrival times.
    pub embed_times: VecDeque<Instant>,
}

/// Max completion samples kept per phase for the rolling-window pace.
const TIMING_WINDOW: usize = 10;

/// Gap below which embed arrivals are treated as one batch burst (the embedding
/// thread emits ~8 ChapterEmbedded events in quick succession).
const EMBED_BURST_GAP: std::time::Duration = std::time::Duration::from_secs(1);

/// Seconds per completion over the rolling window's OWN span:
/// `(newest - oldest) / (len - 1)`. The span is fixed once the window fills, so
/// the pace doesn't creep up when no completions arrive. None when there are
/// fewer than 2 samples.
pub fn rolling_pace(times: &VecDeque<Instant>) -> Option<f64> {
    if times.len() < 2 {
        return None;
    }
    let oldest = *times.front()?;
    let newest = *times.back()?;
    let span = newest.duration_since(oldest);
    Some(span.as_secs_f64() / (times.len() - 1) as f64)
}

/// Whole seconds remaining: `pace × remaining`. None when pace is None or remaining == 0.
pub fn phase_eta(pace: Option<f64>, remaining: usize) -> Option<u64> {
    let pace = pace?;
    if remaining == 0 {
        return None;
    }
    Some((pace * remaining as f64) as u64)
}

#[derive(Debug)]
pub struct JobsState {
    pub jobs: Vec<JobDto>,
    pub filter: JobFilter,
    pub selected: usize,
    pub max_workers: u8,
    pub active_count: u8,
    pub detail_expanded: Option<usize>, // index into jobs[] (unfiltered)
    pub connected: bool,
    /// Server-relative "now" from the latest jobs snapshot (`server_now_ms`).
    /// Job timestamps are monotonic relative to this reference; 0 until the
    /// first snapshot arrives.
    pub server_now_ms: u64,
    /// Rolling fetch/embed completion windows per job (for the two-phase ETA).
    pub timings: HashMap<u64, JobTiming>,
}

impl Default for JobsState {
    fn default() -> Self {
        Self::new()
    }
}

impl JobsState {
    pub fn new() -> Self {
        Self {
            jobs: Vec::new(),
            filter: JobFilter::All,
            selected: 0,
            max_workers: 4,
            active_count: 0,
            detail_expanded: None,
            connected: false,
            server_now_ms: 0,
            timings: HashMap::new(),
        }
    }

    /// Record a fetch completion (JobDetailChanged arrival) for the job.
    pub fn record_fetch(&mut self, id: u64) {
        let timing = self.timings.entry(id).or_default();
        timing.fetch_times.push_back(Instant::now());
        if timing.fetch_times.len() > TIMING_WINDOW {
            timing.fetch_times.pop_front();
        }
    }

    /// Record an embed completion (ChapterEmbedded/ChapterEmbeddedFailed arrival).
    /// The embedding thread emits ~8 events in a burst; arrivals within
    /// [`EMBED_BURST_GAP`] of the last one are treated as the same batch.
    pub fn record_embed(&mut self, id: u64) {
        let timing = self.timings.entry(id).or_default();
        let now = Instant::now();
        if let Some(last) = timing.embed_times.back()
            && now.duration_since(*last) < EMBED_BURST_GAP
        {
            return;
        }
        timing.embed_times.push_back(now);
        if timing.embed_times.len() > TIMING_WINDOW {
            timing.embed_times.pop_front();
        }
    }

    pub fn filtered_jobs(&self) -> Vec<usize> {
        self.jobs
            .iter()
            .enumerate()
            .filter(|(_, j)| match self.filter {
                JobFilter::All => true,
                JobFilter::Running => j.status == "Running",
                JobFilter::Completed => j.status == "Completed" || j.status == "Cancelled",
                JobFilter::Failed => j.status == "Failed",
            })
            .map(|(i, _)| i)
            .collect()
    }

    pub fn selected_job(&self) -> Option<&JobDto> {
        let indices = self.filtered_jobs();
        indices.get(self.selected).map(|&i| &self.jobs[i])
    }

    pub fn selected_job_mut(&mut self) -> Option<&mut JobDto> {
        let indices = self.filtered_jobs();
        indices
            .get(self.selected)
            .copied()
            .map(move |i| &mut self.jobs[i])
    }

    /// Apply a `JobStatusChanged` event to the matching job using the
    /// server-stamped timestamps from the envelope (never the TUI's local
    /// clock — job timestamps are server-relative).
    pub fn update_from_status(
        &mut self,
        id: u64,
        status: &str,
        error: Option<&str>,
        started_at_ms: Option<u64>,
        completed_at_ms: Option<u64>,
    ) {
        if let Some(job) = self.jobs.iter_mut().find(|j| j.id == id) {
            match status {
                "Running" => {
                    if let Some(s) = started_at_ms {
                        job.started_at_ms = Some(s);
                    }
                }
                "Completed" => {
                    if let Some(c) = completed_at_ms {
                        job.completed_at_ms = Some(c);
                    }
                    job.error = None;
                }
                "Failed" => {
                    if let Some(c) = completed_at_ms {
                        job.completed_at_ms = Some(c);
                    }
                    job.error = error.map(|s| s.to_string());
                }
                _ => {}
            }
            job.status = status.to_string();
        }
    }

    pub fn set_outcome(&mut self, id: u64, outcome: JobOutcomeDto) {
        if let Some(job) = self.jobs.iter_mut().find(|j| j.id == id) {
            job.outcome = Some(outcome);
        }
    }

    /// Apply a `JobDetailChanged` event: merge the worker's per-chapter detail
    /// into the job's existing detail. Chapters already marked "Embedded" or
    /// "Failed" by `ChapterEmbedded`/`ChapterEmbeddedFailed` events keep their
    /// status; all other chapters take the incoming status. Chapters not
    /// present in the existing detail are appended.
    pub fn update_from_detail(&mut self, id: u64, detail: Vec<ChapterDetail>) {
        if let Some(job) = self.jobs.iter_mut().find(|j| j.id == id) {
            let merged = match job.detail.take() {
                Some(mut existing) => {
                    for incoming in detail {
                        match existing.iter_mut().find(|c| c.url == incoming.url) {
                            Some(ch) => {
                                if ch.status != "Embedded" && ch.status != "Failed" {
                                    ch.status = incoming.status;
                                }
                            }
                            None => existing.push(incoming),
                        }
                    }
                    existing
                }
                None => detail,
            };
            job.detail = Some(merged);
        }
    }

    /// Apply a `ChapterEmbedded` event: mark the matching chapter in the job's
    /// detail as "Embedded".
    pub fn update_from_embedded(&mut self, id: u64, url: &str) {
        if let Some(job) = self.jobs.iter_mut().find(|j| j.id == id)
            && let Some(detail) = job.detail.as_mut()
            && let Some(ch) = detail.iter_mut().find(|c| c.url == url)
        {
            ch.status = "Embedded".to_string();
        }
    }

    /// Apply a `ChapterEmbeddedFailed` event: mark the matching chapter in the
    /// job's detail as "Failed".
    pub fn update_from_embedded_failed(&mut self, id: u64, url: &str) {
        if let Some(job) = self.jobs.iter_mut().find(|j| j.id == id)
            && let Some(detail) = job.detail.as_mut()
            && let Some(ch) = detail.iter_mut().find(|c| c.url == url)
        {
            ch.status = "Failed".to_string();
        }
    }

    pub fn remove_completed(&mut self) {
        let before = self.jobs.len();
        self.jobs
            .retain(|j| j.status != "Completed" && j.status != "Cancelled");
        let removed = before - self.jobs.len();
        if removed > 0 {
            crate::settings::log(
                crate::settings::LogLevel::Debug,
                "JOBS",
                &format!("Removed {} completed/cancelled jobs", removed),
            );
        }
        self.selected = self.selected.min(self.jobs.len().saturating_sub(1));
    }

    pub fn remove_all(&mut self) {
        let before = self.jobs.len();
        self.jobs.clear();
        self.selected = 0;
        if before > 0 {
            crate::settings::log(
                crate::settings::LogLevel::Debug,
                "JOBS",
                &format!("Cleared all {} jobs", before),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_job(id: u64, status: &str) -> JobDto {
        JobDto {
            id,
            kind: "Scrape".into(),
            status: status.into(),
            target: "http://example.com".into(),
            priority: "Normal".into(),
            chapter_idx: None,
            created_at_ms: 0,
            started_at_ms: None,
            completed_at_ms: None,
            error: None,
            outcome: None,
            detail: None,
        }
    }

    #[test]
    fn test_update_from_status_running_stamps_started_at() {
        let mut jobs = JobsState::new();
        jobs.jobs.push(sample_job(1, "Queued"));
        jobs.update_from_status(1, "Running", None, Some(200), None);
        assert_eq!(jobs.jobs[0].status, "Running");
        assert_eq!(jobs.jobs[0].started_at_ms, Some(200));
        assert!(jobs.jobs[0].completed_at_ms.is_none());
    }

    #[test]
    fn test_update_from_status_failed_stamps_completed_and_error() {
        let mut jobs = JobsState::new();
        jobs.jobs.push(sample_job(1, "Running"));
        jobs.update_from_status(1, "Failed", Some("boom"), None, Some(300));
        assert_eq!(jobs.jobs[0].status, "Failed");
        assert_eq!(jobs.jobs[0].completed_at_ms, Some(300));
        assert_eq!(jobs.jobs[0].error.as_deref(), Some("boom"));
    }

    #[test]
    fn test_update_from_status_completed_clears_error() {
        let mut jobs = JobsState::new();
        let mut job = sample_job(1, "Failed");
        job.error = Some("boom".into());
        jobs.jobs.push(job);
        jobs.update_from_status(1, "Completed", None, None, Some(400));
        assert_eq!(jobs.jobs[0].status, "Completed");
        assert_eq!(jobs.jobs[0].completed_at_ms, Some(400));
        assert!(jobs.jobs[0].error.is_none());
    }

    #[test]
    fn test_update_from_status_unknown_id_is_noop() {
        let mut jobs = JobsState::new();
        jobs.jobs.push(sample_job(1, "Queued"));
        jobs.update_from_status(99, "Running", None, Some(200), None);
        assert_eq!(jobs.jobs[0].status, "Queued");
    }

    #[test]
    fn test_update_from_status_missing_stamps_does_not_stamp() {
        let mut jobs = JobsState::new();
        jobs.jobs.push(sample_job(1, "Queued"));
        // Backward-compat envelope without stamps: status updates, no timestamps.
        jobs.update_from_status(1, "Running", None, None, None);
        assert_eq!(jobs.jobs[0].status, "Running");
        assert!(jobs.jobs[0].started_at_ms.is_none());
    }

    #[test]
    fn test_set_outcome() {
        let mut jobs = JobsState::new();
        jobs.jobs.push(sample_job(1, "Running"));
        let outcome = JobOutcomeDto {
            kind: "BookScraped".into(),
            title: Some("Book".into()),
            chapters: Some(3),
            cover: Some(true),
            content_chars: None,
        };
        jobs.set_outcome(1, outcome.clone());
        assert_eq!(jobs.jobs[0].outcome, Some(outcome));
    }

    #[test]
    fn test_update_from_detail_sets_job_detail() {
        let mut jobs = JobsState::new();
        jobs.jobs.push(sample_job(1, "Running"));
        let detail = vec![
            ChapterDetail {
                title: "1.1 Crappy Monday".into(),
                url: "u1".into(),
                status: "Done".into(),
            },
            ChapterDetail {
                title: "2.1 New Semester".into(),
                url: "u2".into(),
                status: "Pending".into(),
            },
        ];
        jobs.update_from_detail(1, detail.clone());
        assert_eq!(jobs.jobs[0].detail, Some(detail));
    }

    #[test]
    fn test_update_from_detail_unknown_id_is_noop() {
        let mut jobs = JobsState::new();
        jobs.jobs.push(sample_job(1, "Running"));
        jobs.update_from_detail(99, vec![]);
        assert!(jobs.jobs[0].detail.is_none());
    }

    #[test]
    fn test_update_from_detail_preserves_embedded_status() {
        let mut jobs = JobsState::new();
        jobs.jobs.push(sample_job(1, "Running"));
        // Worker's first detail snapshot.
        jobs.update_from_detail(
            1,
            vec![
                ChapterDetail {
                    title: "Ch1".into(),
                    url: "u1".into(),
                    status: "Fetched".into(),
                },
                ChapterDetail {
                    title: "Ch2".into(),
                    url: "u2".into(),
                    status: "Pending".into(),
                },
            ],
        );
        // Embed event flips ch1 to "Embedded".
        jobs.update_from_embedded(1, "u1");
        // A later worker snapshot still reports ch1 as "Fetched" — the merge
        // must keep the event-set "Embedded" status while ch2 takes "Fetched".
        jobs.update_from_detail(
            1,
            vec![
                ChapterDetail {
                    title: "Ch1".into(),
                    url: "u1".into(),
                    status: "Fetched".into(),
                },
                ChapterDetail {
                    title: "Ch2".into(),
                    url: "u2".into(),
                    status: "Fetched".into(),
                },
            ],
        );
        let detail = jobs.jobs[0].detail.as_ref().unwrap();
        assert_eq!(detail[0].status, "Embedded");
        assert_eq!(detail[1].status, "Fetched");
    }

    #[test]
    fn test_update_from_detail_preserves_failed_status() {
        let mut jobs = JobsState::new();
        jobs.jobs.push(sample_job(1, "Running"));
        jobs.update_from_detail(
            1,
            vec![ChapterDetail {
                title: "Ch1".into(),
                url: "u1".into(),
                status: "Fetched".into(),
            }],
        );
        jobs.update_from_embedded_failed(1, "u1");
        // Worker's stale snapshot must not resurrect a failed embed.
        jobs.update_from_detail(
            1,
            vec![ChapterDetail {
                title: "Ch1".into(),
                url: "u1".into(),
                status: "Fetched".into(),
            }],
        );
        assert_eq!(jobs.jobs[0].detail.as_ref().unwrap()[0].status, "Failed");
    }

    #[test]
    fn test_update_from_detail_appends_new_chapters() {
        let mut jobs = JobsState::new();
        jobs.jobs.push(sample_job(1, "Running"));
        jobs.update_from_detail(
            1,
            vec![ChapterDetail {
                title: "Ch1".into(),
                url: "u1".into(),
                status: "Fetched".into(),
            }],
        );
        // A later snapshot adds ch2 — it must be appended, not dropped.
        jobs.update_from_detail(
            1,
            vec![
                ChapterDetail {
                    title: "Ch1".into(),
                    url: "u1".into(),
                    status: "Fetched".into(),
                },
                ChapterDetail {
                    title: "Ch2".into(),
                    url: "u2".into(),
                    status: "Fetched".into(),
                },
            ],
        );
        let detail = jobs.jobs[0].detail.as_ref().unwrap();
        assert_eq!(detail.len(), 2);
        assert_eq!(detail[1].url, "u2");
        assert_eq!(detail[1].status, "Fetched");
    }

    #[test]
    fn test_update_from_embedded_marks_chapter() {
        let mut jobs = JobsState::new();
        let mut job = sample_job(1, "Running");
        job.detail = Some(vec![
            ChapterDetail {
                title: "Ch1".into(),
                url: "u1".into(),
                status: "Fetched".into(),
            },
            ChapterDetail {
                title: "Ch2".into(),
                url: "u2".into(),
                status: "Pending".into(),
            },
        ]);
        jobs.jobs.push(job);
        jobs.update_from_embedded(1, "u1");
        assert_eq!(jobs.jobs[0].detail.as_ref().unwrap()[0].status, "Embedded");
        assert_eq!(jobs.jobs[0].detail.as_ref().unwrap()[1].status, "Pending");
    }

    #[test]
    fn test_update_from_embedded_unknown_url_is_noop() {
        let mut jobs = JobsState::new();
        let mut job = sample_job(1, "Running");
        job.detail = Some(vec![ChapterDetail {
            title: "Ch1".into(),
            url: "u1".into(),
            status: "Fetched".into(),
        }]);
        jobs.jobs.push(job);
        jobs.update_from_embedded(1, "nope");
        assert_eq!(jobs.jobs[0].detail.as_ref().unwrap()[0].status, "Fetched");
    }

    #[test]
    fn test_update_from_embedded_failed_marks_chapter() {
        let mut jobs = JobsState::new();
        let mut job = sample_job(1, "Running");
        job.detail = Some(vec![
            ChapterDetail {
                title: "Ch1".into(),
                url: "u1".into(),
                status: "Fetched".into(),
            },
            ChapterDetail {
                title: "Ch2".into(),
                url: "u2".into(),
                status: "Pending".into(),
            },
        ]);
        jobs.jobs.push(job);
        jobs.update_from_embedded_failed(1, "u1");
        assert_eq!(jobs.jobs[0].detail.as_ref().unwrap()[0].status, "Failed");
        assert_eq!(jobs.jobs[0].detail.as_ref().unwrap()[1].status, "Pending");
    }

    #[test]
    fn test_filtered_jobs_by_status() {
        let mut jobs = JobsState::new();
        jobs.jobs.push(sample_job(1, "Queued"));
        jobs.jobs.push(sample_job(2, "Running"));
        jobs.jobs.push(sample_job(3, "Completed"));
        jobs.jobs.push(sample_job(4, "Failed"));
        jobs.jobs.push(sample_job(5, "Cancelled"));

        jobs.filter = JobFilter::All;
        assert_eq!(jobs.filtered_jobs(), vec![0, 1, 2, 3, 4]);
        jobs.filter = JobFilter::Running;
        assert_eq!(jobs.filtered_jobs(), vec![1]);
        jobs.filter = JobFilter::Completed;
        assert_eq!(jobs.filtered_jobs(), vec![2, 4]);
        jobs.filter = JobFilter::Failed;
        assert_eq!(jobs.filtered_jobs(), vec![3]);
    }

    #[test]
    fn test_remove_completed_keeps_active() {
        let mut jobs = JobsState::new();
        jobs.jobs.push(sample_job(1, "Completed"));
        jobs.jobs.push(sample_job(2, "Running"));
        jobs.jobs.push(sample_job(3, "Cancelled"));
        jobs.remove_completed();
        assert_eq!(jobs.jobs.len(), 1);
        assert_eq!(jobs.jobs[0].id, 2);
    }

    #[test]
    fn test_remove_all_clears() {
        let mut jobs = JobsState::new();
        jobs.jobs.push(sample_job(1, "Queued"));
        jobs.remove_all();
        assert!(jobs.jobs.is_empty());
    }

    #[test]
    fn test_connected_defaults_false() {
        let jobs = JobsState::new();
        assert!(!jobs.connected);
    }

    #[test]
    fn test_server_now_ms_defaults_zero() {
        let jobs = JobsState::new();
        assert_eq!(jobs.server_now_ms, 0);
    }

    #[test]
    fn test_rolling_pace_too_few_samples() {
        let now = Instant::now();
        let mut times = VecDeque::new();
        assert_eq!(rolling_pace(&times), None);
        times.push_back(now);
        assert_eq!(rolling_pace(&times), None);
    }

    #[test]
    fn test_rolling_pace_window_math() {
        let now = Instant::now();
        // 3 samples over 6 seconds → 3s per completion (the window's own span).
        let mut times = VecDeque::new();
        times.push_back(now - std::time::Duration::from_secs(6));
        times.push_back(now - std::time::Duration::from_secs(3));
        times.push_back(now);
        let pace = rolling_pace(&times).unwrap();
        assert!((pace - 3.0).abs() < 1e-6);
    }

    #[test]
    fn test_rolling_pace_uses_window_span_not_external_now() {
        // The pace is the window's own span (newest - oldest) — there is no
        // external `now` to advance, so it never creeps up between completions.
        let now = Instant::now();
        let mut times = VecDeque::new();
        times.push_back(now - std::time::Duration::from_secs(6));
        times.push_back(now - std::time::Duration::from_secs(3));
        times.push_back(now);
        let pace = rolling_pace(&times).unwrap();
        assert!((pace - 3.0).abs() < 1e-6);
    }

    #[test]
    fn test_phase_eta() {
        assert_eq!(phase_eta(Some(2.0), 8), Some(16));
        assert_eq!(phase_eta(Some(2.0), 0), None);
        assert_eq!(phase_eta(None, 8), None);
    }

    #[test]
    fn test_record_fetch_caps_window() {
        let mut jobs = JobsState::new();
        for _ in 0..15 {
            jobs.record_fetch(1);
        }
        assert_eq!(jobs.timings.get(&1).unwrap().fetch_times.len(), 10);
    }

    #[test]
    fn test_record_embed_burst_skips_within_gap() {
        let mut jobs = JobsState::new();
        jobs.record_embed(1);
        jobs.record_embed(1); // within 1s → same burst, skipped
        assert_eq!(jobs.timings.get(&1).unwrap().embed_times.len(), 1);
    }

    #[test]
    fn test_record_embed_batch_gap_records() {
        let mut jobs = JobsState::new();
        jobs.record_embed(1);
        std::thread::sleep(std::time::Duration::from_millis(1100));
        jobs.record_embed(1); // > 1s apart → new batch
        assert_eq!(jobs.timings.get(&1).unwrap().embed_times.len(), 2);
    }
}
