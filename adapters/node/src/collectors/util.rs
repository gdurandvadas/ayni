use ayni_core::{NodePackageManager, RunContext, detect_node_package_manager};
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn package_manager_for_context(context: &RunContext) -> NodePackageManager {
    detect_node_package_manager(&context.workdir).unwrap_or(NodePackageManager::Npm)
}

pub fn run_tool(
    context: &RunContext,
    tool: &str,
    args: &[&str],
) -> Result<std::process::Output, String> {
    let manager = package_manager_for_context(context);
    let (program, argv) = manager.exec_command(tool, args);
    let mut command = Command::new(program.as_str());
    command.current_dir(&context.workdir);
    command.args(argv.iter().map(String::as_str));
    command.output().map_err(|error| {
        format!(
            "failed to execute {} {}: {error}",
            manager.executable(),
            tool
        )
    })
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
