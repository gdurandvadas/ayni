use ayni_core::{CommandFailure, Language, RunContext, SignalKind};
use regex::Regex;
use std::path::{Path, PathBuf};
use std::process::Command;
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

pub fn run_command_for_context(
    context: &RunContext,
    program: &str,
    args: &[String],
) -> Result<std::process::Output, String> {
    let output = Command::new(program)
        .args(args.iter().map(String::as_str))
        .current_dir(&context.execution.exec_cwd)
        .output()
        .map_err(|error| format!("failed to execute {program}: {error}"))?;
    if context.debug {
        eprintln!(
            "[debug] runner={} source={} kind={} resolved_from={} confidence={} ambiguous={}",
            context.execution.runner,
            context.execution.source,
            context.execution.kind,
            context.execution.resolved_from.display(),
            context.execution.confidence,
            context.execution.ambiguous
        );
        eprintln!(
            "[debug] cwd={} command={} {}",
            context.execution.exec_cwd.display(),
            program,
            args.join(" ")
        );
        eprintln!("[debug] exit={}", output.status.code().unwrap_or(-1));
    }
    Ok(output)
}

pub fn command_failure_from_output(
    context: &RunContext,
    kind: SignalKind,
    program: &str,
    args: &[String],
    output: &std::process::Output,
) -> CommandFailure {
    CommandFailure {
        category: failure_category(kind).to_string(),
        classification: String::from("command_error"),
        command: format_command(program, args),
        cwd: context.execution.exec_cwd.display().to_string(),
        exit_code: output.status.code(),
        message: concise_failure_message(output),
    }
}

pub fn setup_failure(
    context: &RunContext,
    command: String,
    message: impl Into<String>,
) -> CommandFailure {
    CommandFailure {
        category: String::from("repo_setup_issue"),
        classification: String::from("missing_report"),
        command,
        cwd: context.execution.exec_cwd.display().to_string(),
        exit_code: None,
        message: message.into(),
    }
}

fn failure_category(kind: SignalKind) -> &'static str {
    match kind {
        SignalKind::Test | SignalKind::Coverage | SignalKind::Mutation => "repo_code_issue",
        SignalKind::Complexity => "repo_setup_issue",
        SignalKind::Size | SignalKind::Deps => "ayni_internal_issue",
    }
}

fn concise_failure_message(output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    format!("{stderr}\n{stdout}")
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| String::from("command failed without stdout/stderr output"))
}

pub fn format_command(program: &str, args: &[String]) -> String {
    if args.is_empty() {
        program.to_string()
    } else {
        format!("{program} {}", args.join(" "))
    }
}

pub fn to_repo_relative_path(repo_root: &Path, candidate: &Path) -> String {
    if let Ok(relative) = candidate.strip_prefix(repo_root) {
        return relative.to_string_lossy().replace('\\', "/");
    }
    if let Ok(canonical_repo_root) = repo_root.canonicalize()
        && let Ok(canonical_candidate) = candidate.canonicalize()
        && let Ok(relative) = canonical_candidate.strip_prefix(canonical_repo_root)
    {
        return relative.to_string_lossy().replace('\\', "/");
    }
    candidate.to_string_lossy().replace('\\', "/")
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

pub fn attr_string(attrs: &str, name: &str) -> Option<String> {
    let pattern = format!(r#"(?:^|\s){name}\s*=\s*"([^"]*)""#);
    let re = Regex::new(&pattern).ok()?;
    re.captures(attrs)
        .and_then(|caps| caps.get(1))
        .map(|value| decode_xml(value.as_str()))
}

pub fn attr_u64(attrs: &str, name: &str) -> Option<u64> {
    attr_string(attrs, name).and_then(|value| value.parse::<u64>().ok())
}

pub fn attr_f64(attrs: &str, name: &str) -> Option<f64> {
    attr_string(attrs, name).and_then(|value| value.parse::<f64>().ok())
}

pub fn decode_xml(value: &str) -> String {
    value
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}
