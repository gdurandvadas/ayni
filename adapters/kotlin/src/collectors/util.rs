use ayni_adapters_common::exec::{context_timeout, run_command};
use ayni_core::{Language, RunContext, SignalKind};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

pub fn gradle_command(
    context: &RunContext,
    kind: SignalKind,
    default_task: &str,
) -> (String, Vec<String>) {
    if let Some(override_cmd) = context.policy.tool_override_for(Language::Kotlin, kind) {
        let args = if override_cmd.args.is_empty() {
            default_gradle_args(default_task)
        } else {
            override_cmd.args.clone()
        };
        return (override_cmd.command.clone(), args);
    }
    (
        context.execution.runner.clone(),
        default_gradle_args(default_task),
    )
}

fn default_gradle_args(task: &str) -> Vec<String> {
    vec![task.to_string(), String::from("--console=plain")]
}

pub fn resolve_gradle_task(
    context: &RunContext,
    preferred_tasks: &[&str],
) -> Result<Option<String>, String> {
    let args = [
        String::from("tasks"),
        String::from("--all"),
        String::from("--quiet"),
    ];
    let timeout = context_timeout(context);
    let output = run_command(
        &context.execution.exec_cwd,
        &context.execution.runner,
        &args,
        timeout,
    )
    .map_err(|error| format!("failed to execute {}: {error}", context.execution.runner))?;
    if !output.status.success() {
        return Ok(None);
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(preferred_tasks
        .iter()
        .find(|task| gradle_task_list_contains(&stdout, task))
        .map(|task| (*task).to_string()))
}

fn gradle_task_list_contains(stdout: &str, task: &str) -> bool {
    let suffix = format!(":{task}");
    stdout.lines().any(|line| {
        let first = line.split_whitespace().next().unwrap_or("");
        first == task || first.ends_with(&suffix)
    })
}

pub fn find_reports(root: &Path, segments: &[&str], extension: &str) -> Vec<PathBuf> {
    let suffix: PathBuf = segments.iter().collect();
    let report_dirs: Vec<PathBuf> = WalkDir::new(root)
        .into_iter()
        .filter_entry(|entry| {
            if entry.depth() == 0 {
                return true;
            }
            !matches!(
                entry.file_name().to_str(),
                Some(".git" | "node_modules" | ".gradle")
            )
        })
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_dir())
        .filter(|entry| entry.path().ends_with(&suffix))
        .map(|entry| entry.path().to_path_buf())
        .collect();
    let mut reports: Vec<PathBuf> = report_dirs
        .into_iter()
        .flat_map(|dir| {
            WalkDir::new(dir)
                .into_iter()
                .filter_map(Result::ok)
                .filter(|entry| entry.file_type().is_file())
                .filter(|entry| {
                    entry.path().extension().and_then(|value| value.to_str()) == Some(extension)
                })
                .map(|entry| entry.path().to_path_buf())
                .collect::<Vec<_>>()
        })
        .collect();
    reports.sort();
    reports.dedup();
    reports
}
