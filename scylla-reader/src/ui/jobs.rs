use crate::state::{JobsState, UiState};
use crate::ui::widgets::hint_line;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use scylla_core::types::{ChapterDetail, JobDto, JobOutcomeDto, job_now_ms};

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

/// Icon for a per-chapter detail status
/// ("Pending" | "Fetched" | "Embedded" | "Failed").
fn detail_icon(status: &str) -> &'static str {
    match status {
        "Embedded" => "✓",
        "Fetched" => "◐",
        "Failed" => "✗",
        _ => "○", // Pending / unknown
    }
}

fn detail_color(status: &str) -> Color {
    match status {
        "Embedded" => Color::Green,
        "Fetched" => Color::Yellow,
        "Failed" => Color::Red,
        _ => Color::DarkGray,
    }
}

/// Estimated seconds remaining for a job with a per-chapter detail.
/// processed = chapters with status "Embedded" (the embedding pace);
/// avg per-chapter time = elapsed / processed; eta = avg * remaining.
/// None when there's no started time, nothing embedded yet, or nothing remaining.
fn estimate_eta(detail: &[ChapterDetail], started_at_ms: Option<u64>, now_ms: u64) -> Option<u64> {
    let processed = detail.iter().filter(|d| d.status == "Embedded").count();
    let remaining = detail.len().saturating_sub(processed);
    let started = started_at_ms?;
    if processed == 0 || remaining == 0 {
        return None;
    }
    let elapsed_ms = now_ms.saturating_sub(started);
    let avg_ms = elapsed_ms / processed as u64;
    let eta_ms = avg_ms * remaining as u64;
    Some(eta_ms / 1000)
}

/// Progress counts for a job's per-chapter detail: `(fetched, embedded, total)`
/// where fetched = "Fetched" + "Embedded" + "Failed" and embedded = "Embedded".
/// None when the detail is empty.
fn detail_progress(detail: &[ChapterDetail]) -> Option<(usize, usize, usize)> {
    if detail.is_empty() {
        return None;
    }
    let fetched = detail
        .iter()
        .filter(|d| d.status == "Fetched" || d.status == "Embedded" || d.status == "Failed")
        .count();
    let embedded = detail.iter().filter(|d| d.status == "Embedded").count();
    Some((fetched, embedded, detail.len()))
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
    let eta = if job.status == "Running" {
        job.detail
            .as_deref()
            .and_then(|d| estimate_eta(d, job.started_at_ms, now_ms))
            .map(|s| format!(" ETA ~{}s", s))
            .unwrap_or_default()
    } else {
        String::new()
    };
    let progress = job
        .detail
        .as_deref()
        .and_then(detail_progress)
        .map(|(fetched, embedded, total)| format!(" ({}/{}/{})", fetched, embedded, total))
        .unwrap_or_default();

    let main = format!(
        "{} {}  {}  {} {}{}{}",
        icon, job.kind, job.target, job.status, elapsed, progress, eta,
    );

    let style = Style::default().fg(color);
    let mut lines = vec![Line::from(vec![Span::styled(main, style)])];

    if expanded {
        lines.push(Line::from(Span::raw(format!("  ID: {}", job.id))));
        if let Some(started) = job.started_at_ms {
            lines.push(Line::from(Span::raw(format!(
                "  Elapsed: {}s",
                now_ms.saturating_sub(started) / 1000
            ))));
        }
        if let Some(outcome) = &job.outcome {
            lines.push(Line::from(Span::styled(
                format!("  Outcome: {}", outcome_line(outcome)),
                Style::default().fg(Color::Cyan),
            )));
        }

        if let Some(detail) = &job.detail {
            if let Some(eta) = estimate_eta(detail, job.started_at_ms, now_ms) {
                let (fetched, embedded, total) =
                    detail_progress(detail).unwrap_or((0, 0, detail.len()));
                lines.push(Line::from(Span::styled(
                    format!(
                        "  ETA: ~{}s ({} fetched, {} embedded / {})",
                        eta, fetched, embedded, total
                    ),
                    Style::default().fg(Color::DarkGray),
                )));
            }
            lines.push(Line::from(Span::styled(
                "  Chapters:".to_string(),
                Style::default().fg(Color::DarkGray),
            )));
            for ch in detail.iter().take(20) {
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("    {} ", detail_icon(&ch.status)),
                        Style::default().fg(detail_color(&ch.status)),
                    ),
                    Span::raw(ch.title.clone()),
                ]));
            }
            if detail.len() > 20 {
                lines.push(Line::from(Span::styled(
                    format!("    ... ({} more)", detail.len() - 20),
                    Style::default().fg(Color::DarkGray),
                )));
            }
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
            detail: None,
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

    #[test]
    fn test_detail_icon_and_color() {
        assert_eq!(detail_icon("Embedded"), "✓");
        assert_eq!(detail_icon("Fetched"), "◐");
        assert_eq!(detail_icon("Failed"), "✗");
        assert_eq!(detail_icon("Pending"), "○");
        assert_eq!(detail_color("Embedded"), Color::Green);
        assert_eq!(detail_color("Fetched"), Color::Yellow);
        assert_eq!(detail_color("Failed"), Color::Red);
        assert_eq!(detail_color("Pending"), Color::DarkGray);
    }

    #[test]
    fn test_jobs_draw_expanded_shows_detail() {
        let mut state = make_state();
        let mut job = sample_job(1, "Running");
        job.kind = "EmbedBatch".into();
        job.detail = Some(vec![
            scylla_core::types::ChapterDetail {
                title: "1.1 Crappy Monday".into(),
                url: "u1".into(),
                status: "Embedded".into(),
            },
            scylla_core::types::ChapterDetail {
                title: "2.1 New Semester".into(),
                url: "u2".into(),
                status: "Pending".into(),
            },
            scylla_core::types::ChapterDetail {
                title: "2.3 Bad Day".into(),
                url: "u3".into(),
                status: "Failed".into(),
            },
        ]);
        state.jobs.jobs.push(job);
        state.jobs.detail_expanded = Some(0);

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                draw(f, f.area(), &mut state.jobs, &state.ui);
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        let content: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(
            content.contains("1.1 Crappy Monday"),
            "content: {}",
            content
        );
        assert!(content.contains("2.1 New Semester"), "content: {}", content);
        assert!(content.contains("2.3 Bad Day"), "content: {}", content);
        assert!(content.contains("Chapters:"), "content: {}", content);
    }

    fn detail(statuses: &[&str]) -> Vec<ChapterDetail> {
        statuses
            .iter()
            .enumerate()
            .map(|(i, s)| ChapterDetail {
                title: format!("Ch{}", i),
                url: format!("u{}", i),
                status: s.to_string(),
            })
            .collect()
    }

    #[test]
    fn test_estimate_eta_no_started_time() {
        let d = detail(&["Embedded", "Pending"]);
        assert_eq!(estimate_eta(&d, None, 100_000), None);
    }

    #[test]
    fn test_estimate_eta_nothing_processed() {
        let d = detail(&["Pending", "Pending"]);
        assert_eq!(estimate_eta(&d, Some(0), 100_000), None);
    }

    #[test]
    fn test_estimate_eta_all_done() {
        let d = detail(&["Embedded", "Embedded"]);
        assert_eq!(estimate_eta(&d, Some(0), 100_000), None);
    }

    #[test]
    fn test_estimate_eta_partial_progress() {
        // 2 of 10 done in 4s → avg 2s per chapter → eta 8 * 2 = 16s.
        let mut d = detail(&["Pending"; 10]);
        d[0].status = "Embedded".into();
        d[1].status = "Embedded".into();
        assert_eq!(estimate_eta(&d, Some(96_000), 100_000), Some(16));
    }

    #[test]
    fn test_estimate_eta_failed_not_counted_as_processed() {
        // Only "Embedded" counts as processed for the ETA: 1 embedded of 4,
        // elapsed 4s → avg 4s → eta 3 * 4 = 12s.
        let mut d = detail(&["Pending"; 4]);
        d[0].status = "Embedded".into();
        d[1].status = "Failed".into();
        assert_eq!(estimate_eta(&d, Some(96_000), 100_000), Some(12));
    }

    #[test]
    fn test_jobs_draw_shows_eta_for_running_job_with_detail() {
        let mut state = make_state();
        let mut job = sample_job(1, "Running");
        job.kind = "EmbedBatch".into();
        job.started_at_ms = Some(job_now_ms().saturating_sub(4000));
        job.detail = Some(detail(&["Embedded", "Embedded", "Pending", "Pending"]));
        state.jobs.jobs.push(job);
        state.jobs.detail_expanded = Some(0);

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                draw(f, f.area(), &mut state.jobs, &state.ui);
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        let content: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(content.contains("ETA ~"), "content: {}", content);
        assert!(
            content.contains("(2 fetched, 2 embedded / 4)"),
            "content: {}",
            content
        );
    }

    #[test]
    fn test_jobs_draw_no_eta_without_detail() {
        let mut state = make_state();
        let mut job = sample_job(1, "Running");
        job.started_at_ms = Some(job_now_ms().saturating_sub(4000));
        state.jobs.jobs.push(job);

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                draw(f, f.area(), &mut state.jobs, &state.ui);
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        let content: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(!content.contains("ETA"), "content: {}", content);
    }

    #[test]
    fn test_detail_progress_empty_is_none() {
        assert_eq!(detail_progress(&[]), None);
    }

    #[test]
    fn test_detail_progress_partial() {
        let mut d = detail(&["Pending"; 10]);
        d[0].status = "Embedded".into();
        d[1].status = "Embedded".into();
        assert_eq!(detail_progress(&d), Some((2, 2, 10)));
    }

    #[test]
    fn test_detail_progress_failed_counts_as_fetched() {
        let mut d = detail(&["Pending"; 4]);
        d[0].status = "Embedded".into();
        d[1].status = "Failed".into();
        // 2 fetched (Embedded + Failed), 1 embedded, 4 total.
        assert_eq!(detail_progress(&d), Some((2, 1, 4)));
    }

    #[test]
    fn test_jobs_draw_main_row_shows_progress() {
        let mut state = make_state();
        let mut job = sample_job(1, "Running");
        job.kind = "EmbedBatch".into();
        job.started_at_ms = Some(job_now_ms().saturating_sub(4000));
        let mut d = detail(&["Pending"; 10]);
        d[0].status = "Embedded".into();
        d[1].status = "Embedded".into();
        job.detail = Some(d);
        state.jobs.jobs.push(job);

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                draw(f, f.area(), &mut state.jobs, &state.ui);
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        let content: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(content.contains("(2/2/10)"), "content: {}", content);
        assert!(content.contains("ETA ~"), "content: {}", content);
    }

    #[test]
    fn test_jobs_draw_main_row_shows_zero_progress_without_eta() {
        let mut state = make_state();
        let mut job = sample_job(1, "Running");
        job.kind = "EmbedBatch".into();
        job.started_at_ms = Some(job_now_ms().saturating_sub(4000));
        job.detail = Some(detail(&["Pending"; 10]));
        state.jobs.jobs.push(job);

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                draw(f, f.area(), &mut state.jobs, &state.ui);
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        let content: String = buf.content().iter().map(|c| c.symbol()).collect();
        // Progress shows even with nothing processed (no ETA yet).
        assert!(content.contains("(0/0/10)"), "content: {}", content);
        assert!(!content.contains("ETA"), "content: {}", content);
    }
}
