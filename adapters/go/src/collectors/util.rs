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
