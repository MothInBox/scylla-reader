//! Jobs page input handler — navigation, cancel, retry, filter, worker adjustment.

use crate::input::keybinds::*;
use crate::state::{JobsState, LibraryState};
use crossterm::event::{KeyCode, KeyEvent};
use scylla_core::types::JobFilter;

pub fn handle_jobs(jobs: &mut JobsState, lib: &mut LibraryState, key: KeyEvent) -> bool {
    let filtered_indices = jobs.filtered_jobs();
    let max_idx = filtered_indices.len().saturating_sub(1);

    match key.code {
        KEY_NAV_DOWN => {
            if max_idx > 0 {
                jobs.selected = (jobs.selected + 1).min(max_idx);
            }
            true
        }
        KEY_NAV_UP => {
            jobs.selected = jobs.selected.saturating_sub(1);
            true
        }
        KEY_JOBS_DETAILS => {
            let current = jobs.detail_expanded;
            let selected_real_idx = filtered_indices.get(jobs.selected).copied();
            if current == selected_real_idx {
                jobs.detail_expanded = None;
            } else {
                jobs.detail_expanded = selected_real_idx;
            }
            true
        }
        KEY_JOBS_CANCEL => {
            if let Some(&real_idx) = filtered_indices.get(jobs.selected) {
                let job_id = jobs.jobs[real_idx].id;
                crate::settings::log(
                    crate::settings::LogLevel::Debug,
                    "JOBS",
                    &format!("Cancel job {}", job_id),
                );
                let base = crate::storage::client::api_base_for(&lib.manager);
                if let Err(e) =
                    crate::storage::client::block_on(crate::storage::client::job_command(
                        &base,
                        "cancel",
                        serde_json::json!({ "id": job_id }),
                    ))
                {
                    crate::settings::log(
                        crate::settings::LogLevel::Error,
                        "JOBS",
                        &format!("Failed to cancel job {}: {}", job_id, e),
                    );
                }
            }
            true
        }
        KEY_JOBS_CANCEL_ALL => {
            crate::settings::log(crate::settings::LogLevel::Debug, "JOBS", "Cancel all jobs");
            let base = crate::storage::client::api_base_for(&lib.manager);
            if let Err(e) = crate::storage::client::block_on(crate::storage::client::job_command(
                &base,
                "cancel-all",
                serde_json::json!({}),
            )) {
                crate::settings::log(
                    crate::settings::LogLevel::Error,
                    "JOBS",
                    &format!("Failed to cancel all jobs: {}", e),
                );
            }
            true
        }
        KEY_JOBS_RETRY => {
            if let Some(&real_idx) = filtered_indices.get(jobs.selected)
                && jobs.jobs[real_idx].status == "Failed"
            {
                let job_id = jobs.jobs[real_idx].id;
                crate::settings::log(
                    crate::settings::LogLevel::Debug,
                    "JOBS",
                    &format!("Retry job {}", job_id),
                );
                let base = crate::storage::client::api_base_for(&lib.manager);
                if let Err(e) =
                    crate::storage::client::block_on(crate::storage::client::job_command(
                        &base,
                        "retry",
                        serde_json::json!({ "id": job_id }),
                    ))
                {
                    crate::settings::log(
                        crate::settings::LogLevel::Error,
                        "JOBS",
                        &format!("Failed to retry job {}: {}", job_id, e),
                    );
                }
            }
            true
        }
        KEY_JOBS_RETRY_ALL => {
            crate::settings::log(
                crate::settings::LogLevel::Debug,
                "JOBS",
                "Retry all failed jobs",
            );
            let base = crate::storage::client::api_base_for(&lib.manager);
            if let Err(e) = crate::storage::client::block_on(crate::storage::client::job_command(
                &base,
                "retry-all",
                serde_json::json!({}),
            )) {
                crate::settings::log(
                    crate::settings::LogLevel::Error,
                    "JOBS",
                    &format!("Failed to retry all jobs: {}", e),
                );
            }
            true
        }
        KeyCode::Char('+') | KeyCode::Char('=') => {
            let new = (jobs.max_workers + 1).min(32);
            crate::settings::log(
                crate::settings::LogLevel::Debug,
                "JOBS",
                &format!("Workers inc to {}", new),
            );
            jobs.max_workers = new;
            lib.settings.max_workers = new;
            lib.settings.save();
            let base = crate::storage::client::api_base_for(&lib.manager);
            if let Err(e) = crate::storage::client::block_on(crate::storage::client::job_command(
                &base,
                "workers",
                serde_json::json!({ "max_workers": new }),
            )) {
                crate::settings::log(
                    crate::settings::LogLevel::Error,
                    "JOBS",
                    &format!("Failed to set workers: {}", e),
                );
            }
            true
        }
        KEY_JOBS_DEC_WORKERS => {
            let new = jobs.max_workers.saturating_sub(1).max(1);
            crate::settings::log(
                crate::settings::LogLevel::Debug,
                "JOBS",
                &format!("Workers dec to {}", new),
            );
            jobs.max_workers = new;
            lib.settings.max_workers = new;
            lib.settings.save();
            let base = crate::storage::client::api_base_for(&lib.manager);
            if let Err(e) = crate::storage::client::block_on(crate::storage::client::job_command(
                &base,
                "workers",
                serde_json::json!({ "max_workers": new }),
            )) {
                crate::settings::log(
                    crate::settings::LogLevel::Error,
                    "JOBS",
                    &format!("Failed to set workers: {}", e),
                );
            }
            true
        }
        KEY_JOBS_FLUSH_COMPLETED => {
            crate::settings::log(
                crate::settings::LogLevel::Debug,
                "JOBS",
                "Flush completed jobs",
            );
            let base = crate::storage::client::api_base_for(&lib.manager);
            if let Err(e) = crate::storage::client::block_on(crate::storage::client::job_command(
                &base,
                "flush-completed",
                serde_json::json!({}),
            )) {
                crate::settings::log(
                    crate::settings::LogLevel::Error,
                    "JOBS",
                    &format!("Failed to flush completed jobs: {}", e),
                );
            }
            jobs.remove_completed();
            true
        }
        KEY_JOBS_FLUSH_ALL => {
            crate::settings::log(crate::settings::LogLevel::Debug, "JOBS", "Flush all jobs");
            let base = crate::storage::client::api_base_for(&lib.manager);
            if let Err(e) = crate::storage::client::block_on(crate::storage::client::job_command(
                &base,
                "flush-all",
                serde_json::json!({}),
            )) {
                crate::settings::log(
                    crate::settings::LogLevel::Error,
                    "JOBS",
                    &format!("Failed to flush all jobs: {}", e),
                );
            }
            jobs.remove_all();
            true
        }
        KeyCode::Char('a') => {
            crate::settings::log(crate::settings::LogLevel::Debug, "JOBS", "Filter: All");
            jobs.filter = JobFilter::All;
            jobs.selected = 0;
            true
        }
        KeyCode::Char('s') => {
            crate::settings::log(crate::settings::LogLevel::Debug, "JOBS", "Filter: Running");
            jobs.filter = JobFilter::Running;
            jobs.selected = 0;
            true
        }
        KeyCode::Char('d') => {
            crate::settings::log(crate::settings::LogLevel::Debug, "JOBS", "Filter: Done");
            jobs.filter = JobFilter::Completed;
            jobs.selected = 0;
            true
        }
        KeyCode::Char('e') => {
            crate::settings::log(crate::settings::LogLevel::Debug, "JOBS", "Filter: Failed");
            jobs.filter = JobFilter::Failed;
            jobs.selected = 0;
            true
        }
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::*;
    use scylla_core::types::JobDto;

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
    fn test_handle_jobs_navigation() {
        let mut state = test_state();
        state.jobs.jobs.push(sample_job(1, "Queued"));
        state.jobs.jobs.push(sample_job(2, "Queued"));
        handle_jobs(&mut state.jobs, &mut state.lib, key_event(KEY_NAV_DOWN));
        assert_eq!(state.jobs.selected, 1);
        handle_jobs(&mut state.jobs, &mut state.lib, key_event(KEY_NAV_UP));
        assert_eq!(state.jobs.selected, 0);
    }

    #[test]
    fn test_handle_jobs_filter_keys() {
        let mut state = test_state();
        handle_jobs(
            &mut state.jobs,
            &mut state.lib,
            key_event(KeyCode::Char('s')),
        );
        assert_eq!(state.jobs.filter, JobFilter::Running);
        handle_jobs(
            &mut state.jobs,
            &mut state.lib,
            key_event(KeyCode::Char('d')),
        );
        assert_eq!(state.jobs.filter, JobFilter::Completed);
        handle_jobs(
            &mut state.jobs,
            &mut state.lib,
            key_event(KeyCode::Char('e')),
        );
        assert_eq!(state.jobs.filter, JobFilter::Failed);
        handle_jobs(
            &mut state.jobs,
            &mut state.lib,
            key_event(KeyCode::Char('a')),
        );
        assert_eq!(state.jobs.filter, JobFilter::All);
    }

    #[test]
    fn test_handle_jobs_workers_inc_dec() {
        let mut state = test_state();
        handle_jobs(
            &mut state.jobs,
            &mut state.lib,
            key_event(KeyCode::Char('+')),
        );
        assert_eq!(state.jobs.max_workers, 5);
        assert_eq!(state.lib.settings.max_workers, 5);
        handle_jobs(
            &mut state.jobs,
            &mut state.lib,
            key_event(KEY_JOBS_DEC_WORKERS),
        );
        assert_eq!(state.jobs.max_workers, 4);
    }

    #[test]
    fn test_handle_jobs_flush_removes_local_jobs() {
        let mut state = test_state();
        state.jobs.jobs.push(sample_job(1, "Completed"));
        state.jobs.jobs.push(sample_job(2, "Running"));
        handle_jobs(
            &mut state.jobs,
            &mut state.lib,
            key_event(KEY_JOBS_FLUSH_COMPLETED),
        );
        assert_eq!(state.jobs.jobs.len(), 1);
        assert_eq!(state.jobs.jobs[0].id, 2);
        handle_jobs(
            &mut state.jobs,
            &mut state.lib,
            key_event(KEY_JOBS_FLUSH_ALL),
        );
        assert!(state.jobs.jobs.is_empty());
    }

    #[test]
    fn test_handle_jobs_details_toggle() {
        let mut state = test_state();
        state.jobs.jobs.push(sample_job(1, "Queued"));
        handle_jobs(&mut state.jobs, &mut state.lib, key_event(KEY_JOBS_DETAILS));
        assert_eq!(state.jobs.detail_expanded, Some(0));
        handle_jobs(&mut state.jobs, &mut state.lib, key_event(KEY_JOBS_DETAILS));
        assert_eq!(state.jobs.detail_expanded, None);
    }
}
