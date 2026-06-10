//! Repository path normalization shared by all adapters.

use std::path::{Path, PathBuf};

/// Renders `candidate` relative to `repo_root` using forward slashes,
/// canonicalizing both sides when a direct prefix strip fails.
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

/// Resolves a possibly-relative path against the repository root.
pub fn resolve_repo_path(repo_root: &Path, value: &str) -> PathBuf {
    let path = Path::new(value);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        repo_root.join(path)
    }
}

/// Normalizes a repo-relative path to canonical POSIX form (`.` for empty,
/// no trailing slashes, forward slashes only).
pub fn canonicalize_relative_posix(value: &str) -> String {
    let mut normalized = value.trim().replace('\\', "/");
    while normalized.ends_with('/') {
        normalized.pop();
    }
    if normalized.is_empty() {
        String::from(".")
    } else {
        normalized
    }
}

#[cfg(test)]
mod tests {
    use super::{canonicalize_relative_posix, resolve_repo_path, to_repo_relative_path};
    use std::path::Path;

    #[test]
    fn relativizes_with_forward_slashes() {
        assert_eq!(
            to_repo_relative_path(Path::new("/repo"), Path::new("/repo/src/main.rs")),
            "src/main.rs"
        );
    }

    #[test]
    fn keeps_outside_paths_verbatim() {
        assert_eq!(
            to_repo_relative_path(Path::new("/repo"), Path::new("/elsewhere/file")),
            "/elsewhere/file"
        );
    }

    #[test]
    fn resolves_relative_against_repo_root() {
        assert_eq!(
            resolve_repo_path(Path::new("/repo"), "src/lib.rs"),
            Path::new("/repo/src/lib.rs")
        );
        assert_eq!(
            resolve_repo_path(Path::new("/repo"), "/abs/file"),
            Path::new("/abs/file")
        );
    }

    #[test]
    fn canonicalizes_posix_form() {
        assert_eq!(canonicalize_relative_posix(""), ".");
        assert_eq!(canonicalize_relative_posix("a\\b//"), "a/b");
        assert_eq!(canonicalize_relative_posix(" pkg/ "), "pkg");
    }
}
