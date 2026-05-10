use ayni_core::{CommandFailure, RunContext, SignalKind};
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn run_tool(workdir: &Path, tool: &str, args: &[&str]) -> Result<std::process::Output, String> {
    let owned = args
        .iter()
        .map(|value| (*value).to_string())
        .collect::<Vec<_>>();
    run_tool_owned(workdir, tool, &owned)
}

pub fn run_tool_owned(
    workdir: &Path,
    tool: &str,
    args: &[String],
) -> Result<std::process::Output, String> {
    Command::new(tool)
        .args(args.iter().map(String::as_str))
        .current_dir(workdir)
        .output()
        .map_err(|error| format!("failed to execute {tool}: {error}"))
}

pub fn run_tool_for_context(
    context: &RunContext,
    tool: &str,
    args: &[String],
) -> Result<std::process::Output, String> {
    let output = run_tool_owned(&context.execution.exec_cwd, tool, args)?;
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
            tool,
            args.join(" ")
        );
        eprintln!("[debug] exit={}", output.status.code().unwrap_or(-1));
    }
    Ok(output)
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

pub fn resolve_repo_path(repo_root: &Path, value: &str) -> PathBuf {
    let path = Path::new(value);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        repo_root.join(path)
    }
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

fn format_command(program: &str, args: &[String]) -> String {
    if args.is_empty() {
        program.to_string()
    } else {
        format!("{program} {}", args.join(" "))
    }
}
