use ayni_adapters_common::{exec, failure};
use ayni_core::{CommandFailure, PythonPackageManager, RunContext, SignalKind};
use std::path::PathBuf;

// Re-export common helpers so existing collector imports (`super::util::*`) are unchanged.
pub use ayni_adapters_common::exec::{format_command, run_command_for_context};
pub use ayni_adapters_common::failure::combined_output;
pub use ayni_adapters_common::paths::to_repo_relative_path;

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

/// Builds a `CommandFailure` using the common category/format helpers but with
/// pytest-specific failure classification and error message extraction.
pub fn command_failure_from_output(
    context: &RunContext,
    kind: SignalKind,
    program: &str,
    args: &[String],
    output: &std::process::Output,
) -> CommandFailure {
    CommandFailure {
        category: failure::failure_category(kind).to_string(),
        classification: classify_failure(output).to_string(),
        command: exec::format_command(program, args),
        cwd: context.execution.exec_cwd.display().to_string(),
        exit_code: output.status.code(),
        message: concise_failure_message(output),
    }
}

/// Thin wrapper so collectors can call `prepare_report_path(context, filename)`
/// without specifying the language string.
pub fn prepare_report_path(context: &RunContext, filename: &str) -> Result<PathBuf, String> {
    ayni_adapters_common::reports::prepare_report_path(context, "python", filename)
}

/// Pytest-specific failure classification: import_error / collection_error / no_tests / command_error.
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

/// Pytest-specific failure message: scans for "E   " / "ERROR " / import error lines
/// before falling back to the first non-empty line.
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

#[cfg(test)]
mod tests {
    use super::prepare_report_path;
    use ayni_core::{AyniPolicy, ExecutionResolution, RunContext, Scope};
    use std::env;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::TempDir;

    #[test]
    fn report_paths_are_absolute_and_stale_reports_are_removed() {
        let temp = TempDir::new_in(env::current_dir().expect("cwd")).expect("tempdir");
        let cwd = env::current_dir().expect("cwd");
        let repo_root = temp
            .path()
            .strip_prefix(&cwd)
            .expect("relative temp path")
            .to_path_buf();
        let context = RunContext {
            repo_root: repo_root.clone(),
            target_root: repo_root.join("packages/config"),
            workdir: repo_root.join("packages/config"),
            policy: AyniPolicy::default(),
            scope: Scope {
                workspace_root: repo_root.to_string_lossy().into_owned(),
                path: Some(String::from("packages/config")),
                package: None,
                file: None,
            },
            diff: None,
            execution: ExecutionResolution::direct(
                "uv",
                PathBuf::from("packages/config"),
                "test",
                100,
            ),
            debug: false,
        };

        let first = prepare_report_path(&context, "coverage.json").expect("first report path");
        assert!(first.is_absolute());
        fs::write(&first, "{}").expect("stale report");

        let second = prepare_report_path(&context, "coverage.json").expect("second report path");
        assert_eq!(second, first);
        assert!(!second.exists());
    }
}
