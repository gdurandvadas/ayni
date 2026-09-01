//! Marker-file root discovery shared by adapter `discovery` modules.

use crate::workspace::is_universal_workspace_state;

use crate::paths::canonicalize_relative_posix;
use std::fs;
use std::path::Path;

/// Walks the repository for directories containing `file_name` and returns
/// their canonical repo-relative paths, sorted and deduplicated. Symlinked
/// directories, universal Ayni/VCS state, and directories whose repo-relative
/// path components match `exclude` are skipped entirely.
pub fn discover_file_parent_roots<F>(repo_root: &Path, file_name: &str, exclude: F) -> Vec<String>
where
    F: Fn(&[&str]) -> bool,
{
    let mut found = Vec::new();
    visit_directory(repo_root, repo_root, file_name, &exclude, &mut found);
    dedupe_and_sort_roots(found)
}

fn visit_directory<F>(
    repo_root: &Path,
    directory: &Path,
    file_name: &str,
    exclude: &F,
    found: &mut Vec<String>,
) where
    F: Fn(&[&str]) -> bool,
{
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            if !directory_is_excluded(repo_root, &path, exclude) {
                visit_directory(repo_root, &path, file_name, exclude, found);
            }
        } else if path.file_name().and_then(|value| value.to_str()) == Some(file_name) {
            record_marker_parent(repo_root, &path, found);
        }
    }
}

fn directory_is_excluded<F>(repo_root: &Path, path: &Path, exclude: &F) -> bool
where
    F: Fn(&[&str]) -> bool,
{
    let Ok(relative) = path.strip_prefix(repo_root) else {
        return true;
    };
    let text = canonicalize_relative_posix(&relative.to_string_lossy());
    let parts = text.split('/').collect::<Vec<_>>();
    is_universal_excluded_dir(&parts) || exclude(&parts)
}

fn record_marker_parent(repo_root: &Path, path: &Path, found: &mut Vec<String>) {
    let Some(parent) = path.parent() else {
        return;
    };
    let Ok(relative) = parent.strip_prefix(repo_root) else {
        return;
    };
    found.push(canonicalize_relative_posix(&relative.to_string_lossy()));
}

/// Sorts and deduplicates discovered roots.
pub fn dedupe_and_sort_roots(mut roots: Vec<String>) -> Vec<String> {
    roots.sort();
    roots.dedup();
    roots
}

/// Component names that should never be descended into for every language.
pub fn is_universal_excluded_dir(parts: &[&str]) -> bool {
    parts.iter().any(|part| is_universal_workspace_state(part))
}

#[cfg(test)]
mod tests {
    use super::{dedupe_and_sort_roots, discover_file_parent_roots};
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

        let roots = discover_file_parent_roots(dir.path(), "go.mod", |parts| {
            parts.contains(&"node_modules")
        });
        assert_eq!(roots, vec![String::from("."), String::from("services/api")]);
    }

    #[cfg(unix)]
    #[test]
    fn skips_symlinked_directories_that_escape_the_repository() {
        use std::os::unix::fs::symlink;

        let repo = TempDir::new().expect("repository");
        let outside = TempDir::new().expect("outside");
        fs::write(outside.path().join("go.mod"), "module escaped\n").expect("outside marker");
        symlink(outside.path(), repo.path().join("escape")).expect("directory symlink");
        symlink(outside.path().join("go.mod"), repo.path().join("go.mod")).expect("marker symlink");

        assert!(discover_file_parent_roots(repo.path(), "go.mod", |_| false).is_empty());
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
