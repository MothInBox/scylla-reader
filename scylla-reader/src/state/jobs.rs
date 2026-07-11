use crate::models::job::{Job, JobFilter, JobOutcome};

#[derive(Debug)]
pub struct JobsState {
    pub jobs: Vec<Job>,
    pub filter: JobFilter,
    pub selected: usize,
    pub max_workers: u8,
    pub active_count: u8,
    pub detail_expanded: Option<usize>, // index into jobs[] (unfiltered)
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
        }
    }

    pub fn filtered_jobs(&self) -> Vec<usize> {
        self.jobs
            .iter()
            .enumerate()
            .filter(|(_, j)| match self.filter {
                JobFilter::All => true,
                JobFilter::Running => matches!(j.status, crate::models::job::JobStatus::Running),
                JobFilter::Completed => {
                    matches!(
                        j.status,
                        crate::models::job::JobStatus::Completed
                            | crate::models::job::JobStatus::Cancelled
                    )
                }
                JobFilter::Failed => matches!(j.status, crate::models::job::JobStatus::Failed(_)),
            })
            .map(|(i, _)| i)
            .collect()
    }

    pub fn selected_job(&self) -> Option<&Job> {
        let indices = self.filtered_jobs();
        indices.get(self.selected).map(|&i| &self.jobs[i])
    }

    pub fn selected_job_mut(&mut self) -> Option<&mut Job> {
        let indices = self.filtered_jobs();
        indices
            .get(self.selected)
            .copied()
            .map(move |i| &mut self.jobs[i])
    }

    pub fn update_from_event(
        &mut self,
        id: crate::models::job::JobId,
        status: crate::models::job::JobStatus,
    ) {
        if let Some(job) = self.jobs.iter_mut().find(|j| j.id == id) {
            match &status {
                crate::models::job::JobStatus::Running => {
                    job.started_at = Some(std::time::Instant::now());
                }
                crate::models::job::JobStatus::Completed
                | crate::models::job::JobStatus::Failed(_) => {
                    job.completed_at = Some(std::time::Instant::now());
                    if let crate::models::job::JobStatus::Failed(e) = &status {
                        job.error = Some(e.clone());
                    }
                }
                _ => {}
            }
            job.status = status;
        }
    }

    pub fn set_outcome(&mut self, id: crate::models::job::JobId, outcome: JobOutcome) {
        if let Some(job) = self.jobs.iter_mut().find(|j| j.id == id) {
            job.outcome = Some(outcome);
        }
    }

    pub fn remove_completed(&mut self) {
        let before = self.jobs.len();
        self.jobs.retain(|j| {
            !matches!(
                j.status,
                crate::models::job::JobStatus::Completed | crate::models::job::JobStatus::Cancelled
            )
        });
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
