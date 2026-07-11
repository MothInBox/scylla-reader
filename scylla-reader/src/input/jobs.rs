//! Jobs page input handler — navigation, cancel, retry, filter, worker adjustment.

use crate::input::keybinds::*;
use crate::messenger::AppCommand;
use crate::models::job::{JobFilter, JobStatus};
use crate::state::{JobsState, LibraryState};
use crossterm::event::{KeyCode, KeyEvent};

pub fn handle_jobs(
    jobs: &mut JobsState,
    lib: &mut LibraryState,
    key: KeyEvent,
    cmd_tx: &std::sync::mpsc::Sender<AppCommand>,
) -> bool {
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
                let _ = cmd_tx.send(AppCommand::CancelJob(job_id));
            }
            true
        }
        KEY_JOBS_CANCEL_ALL => {
            crate::settings::log(
                crate::settings::LogLevel::Debug,
                "JOBS",
                "Cancel all jobs",
            );
            let _ = cmd_tx.send(AppCommand::CancelAll);
            true
        }
        KEY_JOBS_RETRY => {
            if let Some(&real_idx) = filtered_indices.get(jobs.selected)
                && matches!(jobs.jobs[real_idx].status, JobStatus::Failed(_))
            {
                let job_id = jobs.jobs[real_idx].id;
                crate::settings::log(
                    crate::settings::LogLevel::Debug,
                    "JOBS",
                    &format!("Retry job {}", job_id),
                );
                let _ = cmd_tx.send(AppCommand::RetryJob(job_id));
            }
            true
        }
        KEY_JOBS_RETRY_ALL => {
            crate::settings::log(
                crate::settings::LogLevel::Debug,
                "JOBS",
                "Retry all failed jobs",
            );
            let _ = cmd_tx.send(AppCommand::RetryAllFailed);
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
            let _ = cmd_tx.send(AppCommand::SetMaxWorkers(new));
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
            let _ = cmd_tx.send(AppCommand::SetMaxWorkers(new));
            true
        }
        KEY_JOBS_FLUSH_COMPLETED => {
            crate::settings::log(
                crate::settings::LogLevel::Debug,
                "JOBS",
                "Flush completed jobs",
            );
            let _ = cmd_tx.send(AppCommand::FlushCompleted);
            jobs.remove_completed();
            true
        }
        KEY_JOBS_FLUSH_ALL => {
            crate::settings::log(
                crate::settings::LogLevel::Debug,
                "JOBS",
                "Flush all jobs",
            );
            let _ = cmd_tx.send(AppCommand::FlushAll);
            jobs.remove_all();
            true
        }
        KeyCode::Char('a') => {
            crate::settings::log(
                crate::settings::LogLevel::Debug,
                "JOBS",
                "Filter: All",
            );
            jobs.filter = JobFilter::All;
            jobs.selected = 0;
            true
        }
        KeyCode::Char('s') => {
            crate::settings::log(
                crate::settings::LogLevel::Debug,
                "JOBS",
                "Filter: Running",
            );
            jobs.filter = JobFilter::Running;
            jobs.selected = 0;
            true
        }
        KeyCode::Char('d') => {
            crate::settings::log(
                crate::settings::LogLevel::Debug,
                "JOBS",
                "Filter: Done",
            );
            jobs.filter = JobFilter::Completed;
            jobs.selected = 0;
            true
        }
        KeyCode::Char('e') => {
            crate::settings::log(
                crate::settings::LogLevel::Debug,
                "JOBS",
                "Filter: Failed",
            );
            jobs.filter = JobFilter::Failed;
            jobs.selected = 0;
            true
        }
        _ => true,
    }
}
