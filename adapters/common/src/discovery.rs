//! Marker-file root discovery shared by adapter `discovery` modules.

use crate::paths::canonicalize_relative_posix;
use std::fs;
use std::path::Path;

/// Walks the repository for directories containing `file_name` and returns
/// their canonical repo-relative paths, sorted and deduplicated. Directories
/// whose repo-relative path components match `exclude` are skipped entirely.
pub fn discover_file_parent_roots<F>(repo_root: &Path, file_name: &str, exclude: F) -> Vec<String>
where
    F: Fn(&[&str]) -> bool,
{
    let mut found = Vec::new();
    let mut stack = vec![repo_root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(value) => value,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Ok(relative) = path.strip_prefix(repo_root) {
                    let text = canonicalize_relative_posix(&relative.to_string_lossy());
                    let parts: Vec<&str> = text.split('/').collect();
                    if exclude(&parts) {
                        continue;
                    }
                }
                stack.push(path);
                continue;
            }
            if path.file_name().and_then(|v| v.to_str()) != Some(file_name) {
                continue;
            }
            if let Some(parent) = path.parent()
                && let Ok(relative) = parent.strip_prefix(repo_root)
            {
                found.push(canonicalize_relative_posix(&relative.to_string_lossy()));
            }
        }
    }
    dedupe_and_sort_roots(found)
}

/// Sorts and deduplicates discovered roots.
pub fn dedupe_and_sort_roots(mut roots: Vec<String>) -> Vec<String> {
    roots.sort();
    roots.dedup();
    roots
}

/// Component names that should never be descended into for any language.
pub fn is_vcs_or_vendor_dir(parts: &[&str]) -> bool {
    parts
        .iter()
        .any(|part| matches!(*part, ".git" | "node_modules" | ".ayni"))
}

#[cfg(test)]
mod tests {
    use super::{dedupe_and_sort_roots, discover_file_parent_roots, is_vcs_or_vendor_dir};
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn finds_marker_parents_and_skips_excluded_dirs() {
        let dir = TempDir::new().expect("tempdir");
        fs::write(dir.path().join("go.mod"), "module root\n").expect("root marker");
        fs::create_dir_all(dir.path().join("services/api")).expect("api dir");
        fs::write(dir.path().join("services/api/go.mod"), "module api\n").expect("api marker");
        fs::create_dir_all(dir.path().join("node_modules/dep")).expect("vendor dir");
        fs::write(dir.path().join("node_modules/dep/go.mod"), "module dep\n")
            .expect("vendor marker");

        let roots = discover_file_parent_roots(dir.path(), "go.mod", is_vcs_or_vendor_dir);
        assert_eq!(roots, vec![String::from("."), String::from("services/api")]);
    }

    #[test]
    fn dedupes_and_sorts() {
        let roots = dedupe_and_sort_roots(vec![
            String::from("b"),
            String::from("a"),
            String::from("b"),
        ]);
        assert_eq!(roots, vec![String::from("a"), String::from("b")]);
    }
}
