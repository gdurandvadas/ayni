use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::widgets::{Gauge, Paragraph};

use crate::ui::runner::{DashboardView, ToolState, ToolView};

pub fn render(frame: &mut Frame<'_>, view: &DashboardView) {
    let targets = build_targets(&view.tools);
    let rows = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).split(frame.area());
    render_runtime_header(frame, rows[0], &targets);
    render_target_progress_list(frame, rows[1], &targets);
}

fn render_runtime_header(frame: &mut Frame<'_>, area: Rect, targets: &[TargetSummary]) {
    let total = targets.len();
    let running = targets.iter().filter(|target| target.running).count();
    let progress = (overall_progress(targets) * 100.0).round() as usize;
    let line = format!("running {running}/{total} · progress {progress}%");
    frame.render_widget(Paragraph::new(line), area);
}

fn render_target_progress_list(frame: &mut Frame<'_>, area: Rect, targets: &[TargetSummary]) {
    if targets.is_empty() || area.height == 0 {
        frame.render_widget(Paragraph::new("waiting for target results..."), area);
        return;
    }

    let visible_count = (area.height as usize).min(targets.len());
    let start = targets.len().saturating_sub(visible_count);
    let visible = &targets[start..];
    let rows = Layout::vertical(vec![Constraint::Length(1); visible.len()]).split(area);

    for (target, row) in visible.iter().zip(rows.into_iter()) {
        render_target_row(frame, *row, target);
    }
}

fn render_target_row(frame: &mut Frame<'_>, area: Rect, target: &TargetSummary) {
    let ratio = progress_ratio(target.done, target.total);
    let percent = (ratio * 100.0).round() as usize;
    let state = target_state_label(target);
    let color = target_color(target);
    let label = format!("{}: {:>3}% ({state})", target.label, percent);
    let gauge = Gauge::default()
        .ratio(ratio)
        .label(label)
        .gauge_style(Style::default().fg(color))
        .use_unicode(true);
    frame.render_widget(gauge, area);
}

#[derive(Clone, Debug)]
struct TargetSummary {
    label: String,
    total: usize,
    done: usize,
    started: bool,
    running: bool,
    failed: bool,
}

fn build_targets(tools: &[ToolView]) -> Vec<TargetSummary> {
    let mut targets = Vec::<TargetSummary>::new();
    for tool in tools {
        if let Some(target) = targets.iter_mut().find(|target| target.label == tool.language) {
            target.total += 1;
            if matches!(tool.state, ToolState::Done | ToolState::Failed) {
                target.done += 1;
            }
            if tool.state != ToolState::Queued {
                target.started = true;
            }
            if tool.state == ToolState::Running {
                target.running = true;
            }
            if tool.state == ToolState::Failed {
                target.failed = true;
            }
        } else {
            targets.push(TargetSummary {
                label: tool.language.clone(),
                total: 1,
                done: usize::from(matches!(tool.state, ToolState::Done | ToolState::Failed)),
                started: tool.state != ToolState::Queued,
                running: tool.state == ToolState::Running,
                failed: tool.state == ToolState::Failed,
            });
        }
    }
    targets
}

fn target_state_label(target: &TargetSummary) -> &'static str {
    if target.failed && !target.running {
        "failed"
    } else if target.running {
        "running"
    } else if target.done == target.total {
        "done"
    } else if target.started {
        "pending"
    } else {
        "queued"
    }
}

fn target_color(target: &TargetSummary) -> Color {
    match target_state_label(target) {
        "failed" => Color::Red,
        "running" => Color::Cyan,
        "done" => Color::Green,
        "pending" => Color::Yellow,
        _ => Color::DarkGray,
    }
}

fn overall_progress(targets: &[TargetSummary]) -> f64 {
    if targets.is_empty() {
        return 0.0;
    }
    let done = targets.iter().map(|target| target.done).sum::<usize>() as f64;
    let total = targets.iter().map(|target| target.total).sum::<usize>() as f64;
    if total <= f64::EPSILON {
        0.0
    } else {
        (done / total).clamp(0.0, 1.0)
    }
}

fn progress_ratio(done: usize, total: usize) -> f64 {
    if total == 0 {
        0.0
    } else {
        (done as f64 / total as f64).clamp(0.0, 1.0)
    }
}
