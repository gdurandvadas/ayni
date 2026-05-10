use ayni_core::{CommandFailure, PythonPackageManager, RunContext, SignalKind};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn package_manager_for_context(context: &RunContext) -> PythonPackageManager {
    PythonPackageManager::from_executable(&context.execution.runner)
        .unwrap_or(PythonPackageManager::Pip)
}

pub fn run_python_tool(
    context: &RunContext,
    tool: &str,
    args: &[&str],
) -> Result<std::process::Output, String> {
    let manager = package_manager_for_context(context);
    let (program, argv) = manager.run_command(tool, args);
    run_command_for_context(context, &program, &argv)
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

pub fn command_for_override_or_default(
    context: &RunContext,
    kind: SignalKind,
    tool: &str,
    default_args: &[&str],
) -> (String, Vec<String>) {
    if let Some(override_cmd) = context
        .policy
        .tool_override_for(ayni_core::Language::Python, kind)
    {
        let args = if override_cmd.args.is_empty() {
            default_args
                .iter()
                .map(|value| (*value).to_string())
                .collect()
        } else {
            override_cmd.args.clone()
        };
        return (override_cmd.command.clone(), args);
    }
    let manager = package_manager_for_context(context);
    manager.run_command(tool, default_args)
}

pub fn command_failure_from_output(
    context: &RunContext,
    kind: SignalKind,
    program: &str,
    args: &[String],
    output: &std::process::Output,
) -> CommandFailure {
    CommandFailure {
        category: classify_failure_category(kind).to_string(),
        classification: classify_failure(output).to_string(),
        command: format_command(program, args),
        cwd: context.execution.exec_cwd.display().to_string(),
        exit_code: output.status.code(),
        message: concise_failure_message(output),
    }
}

pub fn combined_output(output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !stderr.is_empty() {
        stderr
    } else if !stdout.is_empty() {
        stdout
    } else {
        String::from("command failed without stdout/stderr output")
    }
}

fn concise_failure_message(output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let combined = format!("{stderr}\n{stdout}");
    for line in combined.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("E   ") {
            return trimmed.trim_start_matches("E   ").to_string();
        }
        if trimmed.starts_with("ERROR ") {
            return trimmed.to_string();
        }
        if trimmed.starts_with("ImportError while importing test module") {
            return trimmed.to_string();
        }
    }
    combined
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

fn classify_failure_category(kind: SignalKind) -> &'static str {
    match kind {
        SignalKind::Test | SignalKind::Coverage | SignalKind::Mutation => "repo_code_issue",
        SignalKind::Complexity => "repo_setup_issue",
        SignalKind::Size | SignalKind::Deps => "ayni_internal_issue",
    }
}

fn classify_failure(output: &std::process::Output) -> &'static str {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let combined = format!("{stderr}\n{stdout}");
    if combined.contains("ModuleNotFoundError")
        || combined.contains("ImportError")
        || combined.contains("ERROR collecting")
    {
        "import_error"
    } else if combined.contains("collected 0 items / 1 error")
        || combined.contains("Interrupted: 1 error during collection")
    {
        "collection_error"
    } else if combined.contains("no tests ran") || output.status.code() == Some(5) {
        "no_tests"
    } else {
        "command_error"
    }
}

pub fn ensure_ayni_dir(context: &RunContext) -> Result<PathBuf, String> {
    let dir = context
        .repo_root
        .join(".ayni")
        .join("work")
        .join("python")
        .join(root_slug(context.scope.path.as_deref()));
    fs::create_dir_all(&dir)
        .map_err(|error| format!("failed to create {}: {error}", dir.display()))?;
    Ok(dir)
}

fn root_slug(root: Option<&str>) -> String {
    root.unwrap_or("workspace").replace(['/', '\\'], "__")
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
