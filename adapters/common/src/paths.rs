//! Repository path normalization shared by all adapters.

use ayni_core::AyniPolicy;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

/// Validates the filesystem boundary for every enabled configured language
/// root before adapters inspect or execute it.
///
/// Existing roots are canonicalized so symlink traversal cannot move an
/// operational target outside the canonical repository. Missing roots remain
/// valid inputs for adapter detection and completion reporting.
pub fn validate_configured_root_containment(
    repo_root: &Path,
    policy: &AyniPolicy,
) -> Result<(), String> {
    let canonical_repo = repo_root.canonicalize().map_err(|error| {
        format!(
            "failed to establish configured-root repository containment for {}: {error}",
            repo_root.display()
        )
    })?;
    for language in policy.enabled_languages()? {
        for configured_root in policy.roots_for(language) {
            let candidate = canonical_repo.join(configured_root);
            match fs::symlink_metadata(&candidate) {
                Ok(_) => {}
                Err(error) if error.kind() == ErrorKind::NotFound => continue,
                Err(error) => {
                    return Err(format!(
                        "configured root '{configured_root}' for {language} violates repository containment: cannot inspect {}: {error}",
                        candidate.display()
                    ));
                }
            }
            let resolved = candidate.canonicalize().map_err(|error| {
                format!(
                    "configured root '{configured_root}' for {language} violates repository containment: cannot resolve existing path {}: {error}",
                    candidate.display()
                )
            })?;
            if !resolved.starts_with(&canonical_repo) {
                return Err(format!(
                    "configured root '{configured_root}' for {language} escapes repository containment: {} resolves outside {}",
                    candidate.display(),
                    canonical_repo.display()
                ));
            }
        }
    }
    Ok(())
}

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
    use super::{
        canonicalize_relative_posix, resolve_repo_path, to_repo_relative_path,
        validate_configured_root_containment,
    };
    use ayni_core::AyniPolicy;
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;

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

    fn policy(root: &str) -> AyniPolicy {
        let mut policy = AyniPolicy::default();
        policy.rust.roots = vec![root.to_string()];
        policy
    }

    #[test]
    fn canonical_containment_preserves_inside_and_missing_roots() {
        let tempdir = TempDir::new().expect("tempdir");
        let repository = tempdir.path().join("repository");
        fs::create_dir_all(repository.join("inside")).expect("inside");

        validate_configured_root_containment(&repository.join("."), &policy("inside"))
            .expect("inside root");
        validate_configured_root_containment(&repository, &policy("missing"))
            .expect("missing root remains representable");
    }

    #[cfg(unix)]
    #[test]
    fn canonical_containment_accepts_inside_symlink_and_rejects_escape() {
        use std::os::unix::fs::symlink;

        let tempdir = TempDir::new().expect("tempdir");
        let repository = tempdir.path().join("repository");
        let outside = tempdir.path().join("outside");
        fs::create_dir_all(repository.join("inside")).expect("inside");
        fs::create_dir(&outside).expect("outside");
        symlink(repository.join("inside"), repository.join("inside-link")).expect("inside link");
        symlink(&outside, repository.join("escape-link")).expect("escape link");
        let repository_link = tempdir.path().join("repository-link");
        symlink(&repository, &repository_link).expect("repository link");

        validate_configured_root_containment(&repository_link, &policy("inside-link"))
            .expect("canonical repository and inside symlink");
        let error = validate_configured_root_containment(&repository, &policy("escape-link"))
            .expect_err("escaping symlink");
        assert!(error.contains("configured root 'escape-link'"));
        assert!(error.contains("repository containment"));
    }
}
