use ayni_core::{CommandFailure, NodePackageManager, RunContext, SignalKind};
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn package_manager_for_context(context: &RunContext) -> NodePackageManager {
    NodePackageManager::from_executable(&context.execution.runner)
        .unwrap_or(NodePackageManager::Npm)
}

pub fn run_tool(
    context: &RunContext,
    tool: &str,
    args: &[&str],
) -> Result<std::process::Output, String> {
    let manager = package_manager_for_context(context);
    let (program, argv) = manager.exec_command(tool, args);
    run_command_for_context(context, program.as_str(), &argv).map_err(|error| {
        format!(
            "failed to execute {} {}: {error}",
            manager.executable(),
            tool
        )
    })
}

pub fn run_command_for_context(
    context: &RunContext,
    program: &str,
    args: &[String],
) -> Result<std::process::Output, String> {
    let output = run_command(&context.execution.exec_cwd, program, args)?;
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
        let stdout = String::from_utf8_lossy(&output.stdout);
        if !stdout.trim().is_empty() {
            eprintln!("[debug] stdout:\n{}", stdout.trim_end());
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !stderr.trim().is_empty() {
            eprintln!("[debug] stderr:\n{}", stderr.trim_end());
        }
    }
    Ok(output)
}

pub fn run_command(
    workdir: &Path,
    program: &str,
    args: &[String],
) -> Result<std::process::Output, String> {
    let mut command = Command::new(program);
    command.current_dir(workdir);
    command.args(args.iter().map(String::as_str));
    command
        .output()
        .map_err(|error| format!("failed to execute {program}: {error}"))
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
        classification: failure_classification(output).to_string(),
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

fn failure_classification(output: &std::process::Output) -> &'static str {
    let combined = combined_output(output);
    if combined.contains("Cannot find module") || combined.contains("ERR_MODULE_NOT_FOUND") {
        "import_error"
    } else if combined.contains("No test files found") {
        "no_tests"
    } else {
        "command_error"
    }
}

fn concise_failure_message(output: &std::process::Output) -> String {
    combined_output(output)
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| String::from("command failed without stdout/stderr output"))
}

fn combined_output(output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    format!("{stderr}\n{stdout}")
}

fn format_command(program: &str, args: &[String]) -> String {
    if args.is_empty() {
        program.to_string()
    } else {
        format!("{program} {}", args.join(" "))
    }
}
