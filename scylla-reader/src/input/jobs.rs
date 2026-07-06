//! Jobs page input handler — navigation, cancel, retry, filter, worker adjustment.

use crate::input::keybinds::*;
use crate::messenger::AppCommand;
use crate::models::job::{JobFilter, JobStatus};
use crate::state::AppState;
use crossterm::event::{KeyCode, KeyEvent};

pub fn handle_jobs(
    state: &mut AppState,
    key: KeyEvent,
    cmd_tx: &std::sync::mpsc::Sender<AppCommand>,
) -> bool {
    let filtered_indices = state.jobs_state.filtered_jobs();
    let max_idx = filtered_indices.len().saturating_sub(1);

    match key.code {
        KEY_NAV_DOWN => {
            if max_idx > 0 {
                state.jobs_state.selected = (state.jobs_state.selected + 1).min(max_idx);
            }
            true
        }
        KEY_NAV_UP => {
            state.jobs_state.selected = state.jobs_state.selected.saturating_sub(1);
            true
        }
        KEY_JOBS_DETAILS => {
            let current = state.jobs_state.detail_expanded;
            let selected_real_idx = filtered_indices.get(state.jobs_state.selected).copied();
            if current == selected_real_idx {
                state.jobs_state.detail_expanded = None;
            } else {
                state.jobs_state.detail_expanded = selected_real_idx;
            }
            true
        }
        KEY_JOBS_CANCEL => {
            if let Some(&real_idx) = filtered_indices.get(state.jobs_state.selected) {
                let job_id = state.jobs_state.jobs[real_idx].id;
                let _ = cmd_tx.send(AppCommand::CancelJob(job_id));
            }
            true
        }
        KEY_JOBS_CANCEL_ALL => {
            let _ = cmd_tx.send(AppCommand::CancelAll);
            true
        }
        KEY_JOBS_RETRY => {
            if let Some(&real_idx) = filtered_indices.get(state.jobs_state.selected) {
                if matches!(state.jobs_state.jobs[real_idx].status, JobStatus::Failed(_)) {
                    let job_id = state.jobs_state.jobs[real_idx].id;
                    let _ = cmd_tx.send(AppCommand::RetryJob(job_id));
                }
            }
            true
        }
        KEY_JOBS_RETRY_ALL => {
            let _ = cmd_tx.send(AppCommand::RetryAllFailed);
            true
        }
        KeyCode::Char('+') | KeyCode::Char('=') => {
            let new = (state.jobs_state.max_workers + 1).min(32);
            state.jobs_state.max_workers = new;
            state.settings.max_workers = new;
            state.settings.save();
            let _ = cmd_tx.send(AppCommand::SetMaxWorkers(new));
            true
        }
        KEY_JOBS_DEC_WORKERS => {
            let new = state.jobs_state.max_workers.saturating_sub(1).max(1);
            state.jobs_state.max_workers = new;
            state.settings.max_workers = new;
            state.settings.save();
            let _ = cmd_tx.send(AppCommand::SetMaxWorkers(new));
            true
        }
        KEY_JOBS_FLUSH_COMPLETED => {
            let _ = cmd_tx.send(AppCommand::FlushCompleted);
            state.jobs_state.remove_completed();
            true
        }
        KEY_JOBS_FLUSH_ALL => {
            let _ = cmd_tx.send(AppCommand::FlushAll);
            state.jobs_state.remove_all();
            true
        }
        KeyCode::Char('a') => {
            state.jobs_state.filter = JobFilter::All;
            state.jobs_state.selected = 0;
            true
        }
        KeyCode::Char('s') => {
            state.jobs_state.filter = JobFilter::Running;
            state.jobs_state.selected = 0;
            true
        }
        KeyCode::Char('d') => {
            state.jobs_state.filter = JobFilter::Completed;
            state.jobs_state.selected = 0;
            true
        }
        KeyCode::Char('e') => {
            state.jobs_state.filter = JobFilter::Failed;
            state.jobs_state.selected = 0;
            true
        }
        _ => true,
    }
}
