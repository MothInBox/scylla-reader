use crate::models::job::{Job, JobStatus};
use crate::state::AppState;
use crate::ui::widgets::hint_line;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

pub fn draw(frame: &mut Frame, area: Rect, state: &mut AppState) {
    let hint_height: u16 = if state.show_hints { 3 } else { 1 };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(hint_height),
        ])
        .split(area);

    // Header bar
    let header = format!(
        " Jobs                      Workers: {}/{}  [+][-]",
        state.jobs_state.active_count, state.jobs_state.max_workers,
    );
    frame.render_widget(
        Paragraph::new(header).style(Style::default().fg(Color::Cyan)),
        chunks[0],
    );

    // Job list
    let block = Block::default().borders(Borders::ALL);
    let inner = block.inner(chunks[1]);
    frame.render_widget(block, chunks[1]);

    let filtered_indices = state.jobs_state.filtered_jobs();
    let items: Vec<ListItem> = filtered_indices
        .iter()
        .map(|&i| {
            let job = &state.jobs_state.jobs[i];
            let is_selected = filtered_indices
                .get(state.jobs_state.selected)
                .map(|&si| si == i)
                .unwrap_or(false);
            let is_expanded = state.jobs_state.detail_expanded == Some(i);
            render_job_row(job, is_selected, is_expanded)
        })
        .collect();

    let mut list_state = ratatui::widgets::ListState::default();
    list_state.select(Some(state.jobs_state.selected));

    let list = List::new(items)
        .highlight_style(Style::default().bg(Color::Blue).fg(Color::White))
        .highlight_symbol(" >");

    frame.render_stateful_widget(list, inner, &mut list_state);

    // Footer hints
    if state.show_hints {
        let hint_area = chunks[2];
        let hint_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(hint_area);

        let filter_status = format!(
            " [{}]  {} queued | {} running | {} failed | {} completed",
            state.jobs_state.filter,
            state.jobs_state.jobs.iter().filter(|j| matches!(j.status, JobStatus::Queued)).count(),
            state.jobs_state.jobs.iter().filter(|j| matches!(j.status, JobStatus::Running)).count(),
            state.jobs_state.jobs.iter().filter(|j| matches!(j.status, JobStatus::Failed(_))).count(),
            state.jobs_state.jobs.iter().filter(|j| matches!(j.status, JobStatus::Completed | JobStatus::Cancelled)).count(),
        );
        frame.render_widget(
            Paragraph::new(filter_status).style(Style::default().fg(Color::DarkGray)),
            hint_chunks[0],
        );

        frame.render_widget(
            Paragraph::new(hint_line("Jobs", &[
                ("c", "Cancel"),
                ("C", "CancelAll"),
                ("r", "Retry"),
                ("R", "RetryAll"),
                ("f", "FlushDone"),
                ("F", "FlushAll"),
            ])),
            hint_chunks[1],
        );
        frame.render_widget(
            Paragraph::new(hint_line("View", &[
                ("a", "All"),
                ("r", "Running"),
                ("c", "Done"),
                ("f", "Failed"),
                ("Enter", "Details"),
            ])),
            hint_chunks[2],
        );
    }
}

fn status_icon(status: &JobStatus) -> &'static str {
    match status {
        JobStatus::Queued => "○",
        JobStatus::Running => "●",
        JobStatus::Completed => "✓",
        JobStatus::Failed(_) => "✕",
        JobStatus::Cancelled => "⊘",
    }
}

fn status_color(status: &JobStatus) -> Color {
    match status {
        JobStatus::Queued => Color::White,
        JobStatus::Running => Color::Green,
        JobStatus::Completed => Color::Cyan,
        JobStatus::Failed(_) => Color::Red,
        JobStatus::Cancelled => Color::DarkGray,
    }
}

fn render_job_row<'a>(job: &Job, _selected: bool, expanded: bool) -> ListItem<'a> {
    let icon = status_icon(&job.status);
    let color = status_color(&job.status);
    let elapsed = match &job.status {
        JobStatus::Running => job.started_at
            .map(|s| format!(" {:?}", s.elapsed().as_secs()) + "s")
            .unwrap_or_default(),
        _ => String::new(),
    };

    let main = format!(
        "{} {}  {}  {} {}",
        icon,
        job.kind.label(),
        job.target,
        match &job.status {
            JobStatus::Failed(_e) => format!("FAILED"),
            _ => format!("{:?}", job.status),
        },
        elapsed,
    );

    let style = Style::default().fg(color);
    let mut lines = vec![Line::from(vec![Span::styled(main, style)])];

    if expanded {
        lines.push(Line::from(Span::raw(format!("  ID: {}  Created: {:?} ago", job.id, job.created_at.elapsed().as_secs()))));
        if let Some(started) = job.started_at {
            lines.push(Line::from(Span::raw(format!("  Started: {:?} ago  Elapsed: {:?}s", started.elapsed().as_secs(), job.started_at.unwrap().elapsed().as_secs()))));
        }
        if let Some(e) = &job.error {
            // Show first 200 chars of error, split on newlines
            for line in e.lines().take(3) {
                let truncated = if line.len() > 60 {
                    format!("{}...", &line[..57])
                } else {
                    line.to_string()
                };
                lines.push(Line::from(Span::styled(
                    format!("  Error: {}", truncated),
                    Style::default().fg(Color::Red),
                )));
            }
            if e.lines().count() > 3 {
                lines.push(Line::from(Span::styled(
                    "  ... (more)",
                    Style::default().fg(Color::Red),
                )));
            }
            }
        }

    ListItem::new(lines)
}
