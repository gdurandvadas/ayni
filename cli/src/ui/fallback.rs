use std::time::Duration;

use owo_colors::OwoColorize;

use crate::ui::runner::{ExecContext, Plan, ProgressEvent, ToolState, run_plain};
use crate::ui::{FAIL_RGB, PASS_RGB, color_enabled};

pub fn run<F>(plan: &Plan, exec: F) -> Result<(), String>
where
    F: FnOnce(ExecContext) -> Result<(), String> + Send + 'static,
{
    let color = color_enabled();
    let outcome = run_plain(plan.clone(), exec, |event| match event {
        ProgressEvent::Started { language, name } => {
            print_status(color, "running", &language, &name, None);
        }
        ProgressEvent::Line {
            language,
            name,
            line,
        } => {
            print_status(color, "line", &language, &name, Some(&line));
        }
        ProgressEvent::Finished {
            language,
            name,
            state,
            elapsed,
        } => {
            let state_text = match state {
                ToolState::Done => "passed",
                ToolState::Failed => "failed",
                ToolState::Queued => "queued",
                ToolState::Running => "running",
            };
            print_status(
                color,
                state_text,
                &language,
                &name,
                Some(&format_elapsed(elapsed)),
            );
        }
    })?;
    if outcome.aborted {
        return Err(String::from("operation aborted"));
    }
    Ok(())
}

fn print_status(color: bool, state: &str, language: &str, name: &str, detail: Option<&str>) {
    let base = format!("[{language}] {name} {state}");
    let line = detail.map_or(base.clone(), |d| format!("{base} {d}"));
    if color {
        let styled = match state {
            "passed" => line
                .truecolor(PASS_RGB.0, PASS_RGB.1, PASS_RGB.2)
                .to_string(),
            "failed" => line
                .truecolor(FAIL_RGB.0, FAIL_RGB.1, FAIL_RGB.2)
                .to_string(),
            "running" => line.cyan().to_string(),
            _ => line,
        };
        println!("{styled}");
    } else {
        println!("{line}");
    }
}

fn format_elapsed(duration: Duration) -> String {
    format!("{:.1}s", duration.as_secs_f64())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::runner::{PlanTool, ToolState};

    #[test]
    fn fallback_run_reports_success() {
        let plan = Plan {
            tools: vec![PlanTool {
                id: String::from("rust:test"),
                language: String::from("rust"),
                name: String::from("cargo test"),
            }],
        };
        let result = run(&plan, |ctx| {
            let tool = ctx.tool("rust:test")?;
            tool.started();
            tool.line("running");
            tool.finished(ToolState::Done);
            Ok(())
        });
        assert!(result.is_ok());
    }
}
