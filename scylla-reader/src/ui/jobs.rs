use crate::state::{JobsState, UiState};
use crate::ui::widgets::hint_line;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use scylla_core::types::{JobDto, JobOutcomeDto, job_now_ms};

pub fn draw(frame: &mut Frame, area: Rect, jobs: &mut JobsState, ui: &UiState) {
    let hint_height: u16 = if ui.show_hints { 4 } else { 1 };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(hint_height),
        ])
        .split(area);

    // Header bar
    let connected = if jobs.connected { "●" } else { "○" };
    let header = format!(
        " Jobs  {}  Workers: {}/{}  [+][-]",
        connected, jobs.active_count, jobs.max_workers,
    );
    frame.render_widget(
        Paragraph::new(header).style(Style::default().fg(Color::Cyan)),
        chunks[0],
    );

    // Job list
    let block = Block::default().borders(Borders::ALL);
    let inner = block.inner(chunks[1]);
    frame.render_widget(block, chunks[1]);

    let filtered_indices = jobs.filtered_jobs();
    let items: Vec<ListItem> = filtered_indices
        .iter()
        .map(|&i| {
            let job = &jobs.jobs[i];
            let is_selected = filtered_indices
                .get(jobs.selected)
                .map(|&si| si == i)
                .unwrap_or(false);
            let is_expanded = jobs.detail_expanded == Some(i);
            render_job_row(job, jobs.server_now_ms, is_selected, is_expanded)
        })
        .collect();

    let mut list_state = ratatui::widgets::ListState::default();
    list_state.select(Some(jobs.selected));

    let list = List::new(items)
        .highlight_style(Style::default().bg(Color::Blue).fg(Color::White))
        .highlight_symbol(" >");

    frame.render_stateful_widget(list, inner, &mut list_state);

    // Footer hints
    if ui.show_hints {
        let hint_area = chunks[2];
        let hint_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(hint_area);

        let filter_status = format!(
            " [{}]  {} queued | {} running | {} failed | {} completed",
            jobs.filter,
            jobs.jobs.iter().filter(|j| j.status == "Queued").count(),
            jobs.jobs.iter().filter(|j| j.status == "Running").count(),
            jobs.jobs.iter().filter(|j| j.status == "Failed").count(),
            jobs.jobs
                .iter()
                .filter(|j| j.status == "Completed" || j.status == "Cancelled")
                .count(),
        );
        frame.render_widget(
            Paragraph::new(filter_status).style(Style::default().fg(Color::DarkGray)),
            hint_chunks[0],
        );

        frame.render_widget(
            Paragraph::new(hint_line(
                "Jobs",
                &[
                    ("c", "Cancel"),
                    ("C", "CancelAll"),
                    ("r", "Retry"),
                    ("R", "RetryAll"),
                    ("f", "FlushDone"),
                    ("F", "FlushAll"),
                ],
            )),
            hint_chunks[1],
        );
        frame.render_widget(
            Paragraph::new(hint_line(
                "View",
                &[
                    ("a", "All"),
                    ("s", "Running"),
                    ("d", "Done"),
                    ("e", "Failed"),
                    ("Enter", "Details"),
                ],
            )),
            hint_chunks[2],
        );
        frame.render_widget(
            Paragraph::new(hint_line("Nav", crate::ui::widgets::NAV_HINTS)),
            hint_chunks[3],
        );
    }
}

fn status_icon(status: &str) -> &'static str {
    match status {
        "Queued" => "○",
        "Running" => "●",
        "Completed" => "✓",
        "Failed" => "✕",
        "Cancelled" => "⊘",
        _ => "?",
    }
}

fn status_color(status: &str) -> Color {
    match status {
        "Queued" => Color::White,
        "Running" => Color::Green,
        "Completed" => Color::Cyan,
        "Failed" => Color::Red,
        "Cancelled" => Color::DarkGray,
        _ => Color::White,
    }
}

fn render_job_row<'a>(
    job: &JobDto,
    server_now_ms: u64,
    _selected: bool,
    expanded: bool,
) -> ListItem<'a> {
    // Job timestamps are server-relative; use the snapshot's `server_now_ms`
    // as the "now" reference, falling back to the local monotonic clock until
    // the first snapshot arrives.
    let now_ms = if server_now_ms > 0 {
        server_now_ms
    } else {
        job_now_ms()
    };
    let icon = status_icon(&job.status);
    let color = status_color(&job.status);
    let elapsed = if job.status == "Running" {
        job.started_at_ms
            .map(|s| format!(" {}s", now_ms.saturating_sub(s) / 1000))
            .unwrap_or_default()
    } else {
        String::new()
    };

    let main = format!(
        "{} {}  {}  {} {}",
        icon, job.kind, job.target, job.status, elapsed,
    );

    let style = Style::default().fg(color);
    let mut lines = vec![Line::from(vec![Span::styled(main, style)])];

    if expanded {
        lines.push(Line::from(Span::raw(format!(
            "  ID: {}  Created: {}s ago",
            job.id,
            now_ms.saturating_sub(job.created_at_ms) / 1000
        ))));
        if let Some(started) = job.started_at_ms {
            lines.push(Line::from(Span::raw(format!(
                "  Started: {}s ago  Elapsed: {}s",
                now_ms.saturating_sub(started) / 1000,
                now_ms.saturating_sub(started) / 1000
            ))));
        }
        if let Some(outcome) = &job.outcome {
            lines.push(Line::from(Span::styled(
                format!("  Outcome: {}", outcome_line(outcome)),
                Style::default().fg(Color::Cyan),
            )));
        }

        if let Some(e) = &job.error {
            for line in e.lines().take(10) {
                lines.push(Line::from(Span::styled(
                    format!("  Error: {}", line),
                    Style::default().fg(Color::Red),
                )));
            }
            if e.lines().count() > 10 {
                lines.push(Line::from(Span::styled(
                    "  ... (more)",
                    Style::default().fg(Color::Red),
                )));
            }
        }
    }

    ListItem::new(lines)
}

fn outcome_line(outcome: &JobOutcomeDto) -> String {
    match outcome.kind.as_str() {
        "BookScraped" => format!(
            "Book: {} ({} chapters, cover: {})",
            outcome.title.as_deref().unwrap_or("?"),
            outcome.chapters.unwrap_or(0),
            if outcome.cover.unwrap_or(false) {
                "yes"
            } else {
                "no"
            }
        ),
        "ChapterFetched" => format!(
            "Chapter: {} ({} chars)",
            outcome.title.as_deref().unwrap_or("?"),
            outcome.content_chars.unwrap_or(0)
        ),
        "CoverFetched" => "Cover: fetched".to_string(),
        _ => outcome.kind.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::Library;
    use crate::state::AppState;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn make_state() -> AppState {
        AppState::from_parts(Library::new())
    }

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
        }
    }

    #[test]
    fn test_jobs_draw_renders_without_panic() {
        let mut state = make_state();
        state.jobs.jobs.push(sample_job(1, "Running"));
        state.jobs.jobs.push(sample_job(2, "Completed"));
        state.jobs.connected = true;

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                draw(f, f.area(), &mut state.jobs, &state.ui);
            })
            .unwrap();
    }

    #[test]
    fn test_jobs_draw_shows_connected_indicator() {
        let mut state = make_state();
        state.jobs.connected = true;
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                draw(f, f.area(), &mut state.jobs, &state.ui);
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        let content: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(content.contains('●'));
    }

    #[test]
    fn test_status_icon_and_color() {
        assert_eq!(status_icon("Queued"), "○");
        assert_eq!(status_icon("Running"), "●");
        assert_eq!(status_icon("Completed"), "✓");
        assert_eq!(status_icon("Failed"), "✕");
        assert_eq!(status_icon("Cancelled"), "⊘");
        assert_eq!(status_color("Running"), Color::Green);
        assert_eq!(status_color("Failed"), Color::Red);
    }

    #[test]
    fn test_outcome_line_variants() {
        let book = JobOutcomeDto {
            kind: "BookScraped".into(),
            title: Some("Book".into()),
            chapters: Some(3),
            cover: Some(true),
            content_chars: None,
        };
        assert_eq!(outcome_line(&book), "Book: Book (3 chapters, cover: yes)");
        let chapter = JobOutcomeDto {
            kind: "ChapterFetched".into(),
            title: Some("Ch1".into()),
            chapters: None,
            cover: None,
            content_chars: Some(500),
        };
        assert_eq!(outcome_line(&chapter), "Chapter: Ch1 (500 chars)");
        let cover = JobOutcomeDto {
            kind: "CoverFetched".into(),
            title: None,
            chapters: None,
            cover: None,
            content_chars: None,
        };
        assert_eq!(outcome_line(&cover), "Cover: fetched");
    }
}
