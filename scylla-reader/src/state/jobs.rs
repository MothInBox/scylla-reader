use scylla_core::types::{ChapterDetail, JobDto, JobFilter, JobOutcomeDto};

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

    /// Apply a `JobDetailChanged` event: set the per-chapter detail on the
    /// matching `EmbedBatch` job.
    pub fn update_from_detail(&mut self, id: u64, detail: Vec<ChapterDetail>) {
        if let Some(job) = self.jobs.iter_mut().find(|j| j.id == id) {
            job.detail = Some(detail);
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
}
