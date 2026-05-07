use ayni_core::{PythonPackageManager, RunContext, detect_python_package_manager};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn package_manager_for_context(context: &RunContext) -> PythonPackageManager {
    detect_python_package_manager(&context.workdir).unwrap_or(PythonPackageManager::Pip)
}

pub fn run_python_tool(
    context: &RunContext,
    tool: &str,
    args: &[&str],
) -> Result<std::process::Output, String> {
    let manager = package_manager_for_context(context);
    let (program, argv) = manager.run_command(tool, args);
    run_command(&context.workdir, &program, &argv)
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
    kind: ayni_core::SignalKind,
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

pub fn ensure_ayni_dir(context: &RunContext) -> Result<PathBuf, String> {
    let dir = context.workdir.join(".ayni");
    fs::create_dir_all(&dir)
        .map_err(|error| format!("failed to create {}: {error}", dir.display()))?;
    Ok(dir)
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
