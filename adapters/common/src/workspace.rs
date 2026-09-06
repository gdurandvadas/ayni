//! Universal repository state omitted from managed workspace copies and source provenance.
//! Logical workspace enumeration shared by managed copies and provenance.
//!
//! Only Ayni and VCS state are universal exclusions. Language adapters retain
//! ownership of ecosystem-specific generated-directory semantics, and ordinary
//! source directories are preserved regardless of their basename.

use crate::exec::run_command;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const UNIVERSAL_WORKSPACE_STATE_NAMES: &[&str] = &[".ayni", ".git"];

#[must_use]
pub fn is_universal_workspace_state(name: &str) -> bool {
    UNIVERSAL_WORKSPACE_STATE_NAMES.contains(&name)
}

#[must_use]
pub fn has_git_ancestor(root: &Path) -> bool {
    root.ancestors()
        .any(|directory| directory.join(".git").exists())
}

/// Enumerate a non-Git workspace without following directory symlinks.
/// Universal state is omitted and enumeration stops at the caller's limit.
pub fn filesystem_workspace_entries(
    root: &Path,
    entry_limit: usize,
) -> Result<Vec<PathBuf>, String> {
    fn walk(
        root: &Path,
        directory: &Path,
        entry_limit: usize,
        files: &mut Vec<PathBuf>,
    ) -> Result<(), String> {
        let mut entries = std::fs::read_dir(directory)
            .map_err(|error| format!("failed to enumerate {}: {error}", directory.display()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("failed to enumerate {}: {error}", directory.display()))?;
        entries.sort_by_key(std::fs::DirEntry::file_name);
        for entry in entries {
            let path = entry.path();
            if is_universal_workspace_state(&entry.file_name().to_string_lossy()) {
                continue;
            }
            let file_type = entry
                .file_type()
                .map_err(|error| format!("failed to inspect {}: {error}", path.display()))?;
            if file_type.is_dir() {
                walk(root, &path, entry_limit, files)?;
            } else {
                files.push(
                    path.strip_prefix(root)
                        .map_err(|_| format!("workspace path {} escaped its root", path.display()))?
                        .to_path_buf(),
                );
                if files.len() > entry_limit {
                    return Err(format!(
                        "filesystem workspace exceeds the {entry_limit} entry safety limit"
                    ));
                }
            }
        }
        Ok(())
    }

    let mut files = Vec::new();
    walk(root, root, entry_limit, &mut files)?;
    Ok(files)
}

/// Enumerate the tracked and unignored untracked files that form a logical Git
/// workspace. Deleted index entries are omitted; directories (for example
/// initialized submodules) fail explicitly rather than being copied recursively
/// without their own ignore contract.
pub fn git_workspace_entries(root: &Path, timeout: Duration) -> Result<Vec<PathBuf>, String> {
    let args = [
        String::from("ls-files"),
        String::from("--cached"),
        String::from("--others"),
        String::from("--exclude-standard"),
        String::from("-z"),
        String::from("--"),
        String::from("."),
    ];
    let output = run_command(root, "git", &args, timeout)
        .map_err(|error| format!("failed to enumerate Git workspace files: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "failed to enumerate Git workspace files: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    let mut files = Vec::new();
    for raw in output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
    {
        if let Some(path) = validate_git_workspace_entry(root, raw)? {
            files.push(path);
        }
    }
    files.sort();
    files.dedup();
    Ok(files)
}

fn validate_git_workspace_entry(root: &Path, raw: &[u8]) -> Result<Option<PathBuf>, String> {
    let relative = std::str::from_utf8(raw)
        .map_err(|_| String::from("Git workspace requires UTF-8 repository paths"))?;
    // Git reports untracked nested repositories with a trailing slash. Normalize
    // that directory marker before excluding Ayni's generated provider caches;
    // ordinary nested repositories still fail the directory check below.
    let relative = relative.strip_suffix('/').unwrap_or(relative);
    let components = relative.split('/').collect::<Vec<_>>();
    let invalid = components.is_empty()
        || components
            .iter()
            .any(|component| component.is_empty() || matches!(*component, "." | ".."))
        || relative.starts_with('/');
    if invalid {
        return Err(format!(
            "Git returned a non-normalized workspace path: {relative:?}"
        ));
    }
    if components
        .iter()
        .any(|component| is_universal_workspace_state(component))
    {
        return Ok(None);
    }
    match std::fs::symlink_metadata(root.join(relative)) {
        Ok(metadata) if metadata.is_dir() => Err(format!(
            "Git workspace entry {relative} is a directory; initialize nested repositories separately"
        )),
        Ok(_) => Ok(Some(PathBuf::from(relative))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!(
            "failed to inspect Git workspace file {relative}: {error}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        UNIVERSAL_WORKSPACE_STATE_NAMES, filesystem_workspace_entries, git_workspace_entries,
        has_git_ancestor, is_universal_workspace_state, validate_git_workspace_entry,
    };
    use std::fs;

    #[test]
    fn git_enumeration_excludes_nested_repositories_in_generated_state() {
        let root = tempfile::TempDir::new().expect("workspace");
        for relative in ["", ".ayni/cache/provider"] {
            let directory = root.path().join(relative);
            fs::create_dir_all(&directory).expect("directory");
            assert!(
                std::process::Command::new("git")
                    .args(["init", "--quiet"])
                    .arg(&directory)
                    .status()
                    .expect("git init")
                    .success()
            );
        }
        fs::write(root.path().join("source.txt"), "source").expect("source");
        fs::write(root.path().join(".ayni/cache/provider/input"), "cache").expect("cache");
        let entries = git_workspace_entries(root.path(), std::time::Duration::from_secs(10))
            .expect("generated nested repository is excluded");
        assert_eq!(entries, [std::path::PathBuf::from("source.txt")]);
        fs::rename(
            root.path().join(".ayni/cache/provider"),
            root.path().join("nested"),
        )
        .expect("ordinary nested repository");
        assert!(
            git_workspace_entries(root.path(), std::time::Duration::from_secs(10))
                .unwrap_err()
                .contains("is a directory")
        );
    }

    #[test]
    fn git_directory_markers_do_not_allow_non_normalized_paths() {
        let root = tempfile::TempDir::new().expect("workspace");
        for path in [
            "/",
            "../outside/",
            "./source/",
            ".ayni//cache/",
            ".ayni/../source/",
        ] {
            assert!(validate_git_workspace_entry(root.path(), path.as_bytes()).is_err());
        }
    }

    #[test]
    fn filesystem_enumeration_is_bounded_and_skips_universal_state() {
        let root = tempfile::TempDir::new().expect("workspace");
        fs::create_dir_all(root.path().join("target/debug")).expect("generated-like directory");
        fs::create_dir_all(root.path().join(".ayni/last")).expect("Ayni state");
        fs::write(root.path().join("target/debug/source.rs"), "source\n").expect("source");
        fs::write(root.path().join("visible.txt"), "visible\n").expect("visible");
        fs::write(root.path().join(".ayni/last/signals.json"), "state\n").expect("state");

        let files = filesystem_workspace_entries(root.path(), 2).expect("bounded workspace");
        assert_eq!(
            files,
            [
                std::path::PathBuf::from("target/debug/source.rs"),
                std::path::PathBuf::from("visible.txt"),
            ]
        );
        assert!(filesystem_workspace_entries(root.path(), 1).is_err());

        fs::create_dir(root.path().join(".git")).expect("Git marker");
        let nested = root.path().join("target/debug");
        assert!(has_git_ancestor(&nested));
    }

    #[test]
    fn universal_workspace_state_names_are_sorted_and_unique() {
        assert!(
            UNIVERSAL_WORKSPACE_STATE_NAMES
                .windows(2)
                .all(|names| names[0] < names[1])
        );
        assert!(is_universal_workspace_state(".ayni"));
        assert!(is_universal_workspace_state(".git"));
        for name in [
            ".gradle",
            ".svelte-kit",
            ".venv",
            "__pycache__",
            "build",
            "coverage",
            "node_modules",
            "target",
        ] {
            assert!(!is_universal_workspace_state(name));
        }
    }
}
