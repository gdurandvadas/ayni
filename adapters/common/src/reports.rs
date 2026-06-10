//! Per-run working directories for adapter report files (`.ayni/work/...`).

use ayni_core::RunContext;
use std::fs;
use std::io;
use std::path::PathBuf;

/// Ensures `.ayni/work/<language>/<root-slug>` exists and returns its
/// canonical absolute path.
pub fn ensure_work_dir(context: &RunContext, language: &str) -> Result<PathBuf, String> {
    let dir = context
        .repo_root
        .join(".ayni")
        .join("work")
        .join(language)
        .join(root_slug(context.scope.path.as_deref()));
    fs::create_dir_all(&dir)
        .map_err(|error| format!("failed to create {}: {error}", dir.display()))?;
    dir.canonicalize()
        .map_err(|error| format!("failed to resolve {}: {error}", dir.display()))
}

/// Returns an absolute report path inside the work dir, removing any stale
/// file from a previous run so collectors never read leftover reports.
pub fn prepare_report_path(
    context: &RunContext,
    language: &str,
    filename: &str,
) -> Result<PathBuf, String> {
    let path = ensure_work_dir(context, language)?.join(filename);
    match fs::remove_file(&path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("failed to remove {}: {error}", path.display())),
    }
    Ok(path)
}

fn root_slug(root: Option<&str>) -> String {
    root.unwrap_or("workspace").replace(['/', '\\'], "__")
}
