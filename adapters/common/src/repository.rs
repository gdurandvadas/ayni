//! Containment-safe repository input reads for adapter discovery.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

pub fn contained_existing_path(repo_root: &Path, path: &Path) -> Result<PathBuf, String> {
    let canonical_repo = repo_root.canonicalize().map_err(|error| {
        format!(
            "failed to establish repository containment for {}: {error}",
            repo_root.display()
        )
    })?;
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        canonical_repo.join(path)
    };
    let resolved = candidate.canonicalize().map_err(|error| {
        format!(
            "failed to resolve repository input {}: {error}",
            candidate.display()
        )
    })?;
    if !resolved.starts_with(&canonical_repo) {
        return Err(format!(
            "repository input {} escapes repository containment",
            candidate.display()
        ));
    }
    Ok(resolved)
}

pub fn read_contained_string(repo_root: &Path, path: &Path) -> Result<String, String> {
    let resolved = contained_existing_path(repo_root, path)?;
    fs::read_to_string(&resolved)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))
}

pub fn read_optional_contained_string(
    repo_root: &Path,
    path: &Path,
) -> Result<Option<String>, String> {
    match fs::symlink_metadata(path) {
        Ok(_) => read_contained_string(repo_root, path).map(Some),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("failed to inspect {}: {error}", path.display())),
    }
}

pub fn read_optional_contained_bytes(
    repo_root: &Path,
    path: &Path,
) -> Result<Option<Vec<u8>>, String> {
    match fs::symlink_metadata(path) {
        Ok(_) => {
            let resolved = contained_existing_path(repo_root, path)?;
            fs::read(&resolved)
                .map(Some)
                .map_err(|error| format!("failed to read {}: {error}", path.display()))
        }
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("failed to inspect {}: {error}", path.display())),
    }
}

pub fn repository_relative(repo_root: &Path, path: &Path) -> Result<String, String> {
    let relative = path.strip_prefix(repo_root).map_err(|_| {
        format!(
            "repository input {} is not below {}",
            path.display(),
            repo_root.display()
        )
    })?;
    let value = relative.to_string_lossy().replace('\\', "/");
    Ok(if value.is_empty() {
        String::from(".")
    } else {
        value
    })
}

#[cfg(test)]
mod tests {
    use super::{read_contained_string, repository_relative};
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn reads_contained_file_and_relativizes_it() {
        let fixture = TempDir::new().expect("fixture");
        let file = fixture.path().join("input");
        fs::write(&file, "value").expect("input");
        assert_eq!(
            read_contained_string(fixture.path(), &file).expect("read"),
            "value"
        );
        assert_eq!(
            repository_relative(fixture.path(), &file).expect("relative"),
            "input"
        );
    }

    #[cfg(unix)]
    #[test]
    fn rejects_file_symlink_escape() {
        use std::os::unix::fs::symlink;
        let fixture = TempDir::new().expect("fixture");
        let repository = fixture.path().join("repository");
        fs::create_dir(&repository).expect("repository");
        let outside = fixture.path().join("outside");
        fs::write(&outside, "secret").expect("outside");
        let link = repository.join("input");
        symlink(&outside, &link).expect("link");
        let error = read_contained_string(&repository, &link).expect_err("escape");
        assert!(error.contains("escapes repository containment"));
    }
}
