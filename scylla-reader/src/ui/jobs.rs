use crate::state::jobs::JobTiming;
use crate::state::{JobsState, UiState};
use crate::ui::widgets::hint_line;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use scylla_core::types::{ChapterDetail, JobDto, JobOutcomeDto, job_now_ms};

pub fn draw(
    frame: &mut Frame,
    area: Rect,
    jobs: &mut JobsState,
    ui: &UiState,
    rate_limit_secs: u64,
) {
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
            let timing = jobs.timings.get(&job.id);
            render_job_row(
                job,
                jobs.server_now_ms,
                timing,
                rate_limit_secs,
                is_selected,
                is_expanded,
            )
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

/// Two-phase ETA for a job: the max of the fetch and embed phase estimates.
/// Returns `(max_eta, fetch_eta, embed_eta)` so the expanded view can show both.
/// The fetch pace is floored by the settings' rate-limit delay; the embed pace
/// is per batch (the server embeds in batches of 8).
fn job_eta(
    detail: &[ChapterDetail],
    timing: Option<&JobTiming>,
    rate_limit_secs: u64,
) -> (Option<u64>, Option<u64>, Option<u64>) {
    let (fetched, embedded, total) = detail_progress(detail).unwrap_or((0, 0, detail.len()));
    let fetch_pace = timing
        .and_then(|t| crate::state::jobs::rolling_pace(&t.fetch_times))
        .map(|p| p.max(rate_limit_secs as f64));
    let fetch_eta = crate::state::jobs::phase_eta(fetch_pace, total.saturating_sub(fetched));
    let embed_batch_pace = timing.and_then(|t| crate::state::jobs::rolling_pace(&t.embed_times));
    // The server's embedding thread processes batches of 8 (EMBED_BATCH_SIZE).
    let remaining_batches = (total.saturating_sub(embedded)).div_ceil(8);
    let embed_eta = crate::state::jobs::phase_eta(embed_batch_pace, remaining_batches);
    let max_eta = match (fetch_eta, embed_eta) {
        (Some(f), Some(e)) => Some(f.max(e)),
        (Some(f), None) => Some(f),
        (None, Some(e)) => Some(e),
        (None, None) => None,
    };
    (max_eta, fetch_eta, embed_eta)
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
    timing: Option<&JobTiming>,
    rate_limit_secs: u64,
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
    let (max_eta, fetch_eta, embed_eta) = if job.status == "Running" {
        job.detail
            .as_deref()
            .map(|d| job_eta(d, timing, rate_limit_secs))
            .unwrap_or((None, None, None))
    } else {
        (None, None, None)
    };
    let eta = max_eta.map(|s| format!(" ETA ~{}s", s)).unwrap_or_default();
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
            if let Some(max_eta) = max_eta {
                let eta_line = match (fetch_eta, embed_eta) {
                    (Some(f), Some(e)) => {
                        format!("  ETA: ~{}s (fetch ~{}s · embed ~{}s)", max_eta, f, e)
                    }
                    (Some(f), None) => format!("  ETA: ~{}s (fetch ~{}s)", max_eta, f),
                    (None, Some(e)) => format!("  ETA: ~{}s (embed ~{}s)", max_eta, e),
                    (None, None) => format!("  ETA: ~{}s", max_eta),
                };
                lines.push(Line::from(Span::styled(
                    eta_line,
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
    use std::time::Instant;

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
                draw(f, f.area(), &mut state.jobs, &state.ui, 2);
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
                draw(f, f.area(), &mut state.jobs, &state.ui, 2);
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
                draw(f, f.area(), &mut state.jobs, &state.ui, 2);
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

    fn timing(fetch_secs_ago: &[u64], embed_secs_ago: &[u64]) -> JobTiming {
        let now = Instant::now();
        JobTiming {
            fetch_times: fetch_secs_ago
                .iter()
                .map(|&s| now - std::time::Duration::from_secs(s))
                .collect(),
            embed_times: embed_secs_ago
                .iter()
                .map(|&s| now - std::time::Duration::from_secs(s))
                .collect(),
        }
    }

    #[test]
    fn test_job_eta_no_timing() {
        let d = detail(&["Fetched", "Pending"]);
        assert_eq!(job_eta(&d, None, 2), (None, None, None));
    }

    #[test]
    fn test_job_eta_fetch_only() {
        // 2 fetched of 10, fetch pace 3s → fetch eta 8 * 3 = 24s; no embed pace.
        let mut d = detail(&["Pending"; 10]);
        d[0].status = "Fetched".into();
        d[1].status = "Fetched".into();
        let t = timing(&[6, 3, 0], &[]);
        assert_eq!(job_eta(&d, Some(&t), 2), (Some(24), Some(24), None));
    }

    #[test]
    fn test_job_eta_both_phases_max() {
        // 4 fetched, 2 embedded of 10. fetch pace 3s → 6 * 3 = 18s;
        // embed pace 2s per batch, 1 batch remaining → 2s; max = 18s.
        let mut d = detail(&["Pending"; 10]);
        d[0].status = "Fetched".into();
        d[1].status = "Fetched".into();
        d[2].status = "Embedded".into();
        d[3].status = "Embedded".into();
        let t = timing(&[6, 3, 0], &[4, 2, 0]);
        assert_eq!(job_eta(&d, Some(&t), 2), (Some(18), Some(18), Some(2)));
    }

    #[test]
    fn test_job_eta_rate_limit_floor() {
        // fetch pace 3s floored to rate_limit 5s → 8 * 5 = 40s.
        let mut d = detail(&["Pending"; 10]);
        d[0].status = "Fetched".into();
        d[1].status = "Fetched".into();
        let t = timing(&[6, 3, 0], &[]);
        let (_, fetch_eta, _) = job_eta(&d, Some(&t), 5);
        assert_eq!(fetch_eta, Some(40));
    }

    #[test]
    fn test_job_eta_all_done() {
        let d = detail(&["Embedded"; 10]);
        let t = timing(&[6, 3, 0], &[4, 2, 0]);
        assert_eq!(job_eta(&d, Some(&t), 2), (None, None, None));
    }

    #[test]
    fn test_job_eta_embed_uses_batches() {
        // 2 embedded of 20 → 18 remaining → 3 batches (ceil(18/8)).
        // embed pace 2s per batch → 6s.
        let mut d = detail(&["Pending"; 20]);
        d[0].status = "Embedded".into();
        d[1].status = "Embedded".into();
        let t = timing(&[], &[4, 2, 0]);
        let (_, _, embed_eta) = job_eta(&d, Some(&t), 2);
        assert_eq!(embed_eta, Some(6));
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
        state.jobs.timings.insert(1, timing(&[4, 0], &[4, 0]));

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                draw(f, f.area(), &mut state.jobs, &state.ui, 2);
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        let content: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(content.contains("ETA ~"), "content: {}", content);
        assert!(
            content.contains("(fetch ~8s · embed ~4s)"),
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
                draw(f, f.area(), &mut state.jobs, &state.ui, 2);
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
        state.jobs.timings.insert(1, timing(&[4, 0], &[4, 0]));

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                draw(f, f.area(), &mut state.jobs, &state.ui, 2);
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
                draw(f, f.area(), &mut state.jobs, &state.ui, 2);
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        let content: String = buf.content().iter().map(|c| c.symbol()).collect();
        // Progress shows even with nothing processed (no ETA yet).
        assert!(content.contains("(0/0/10)"), "content: {}", content);
        assert!(!content.contains("ETA"), "content: {}", content);
    }
}
