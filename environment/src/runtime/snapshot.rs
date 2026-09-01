use crate::BackendError;
use ayni_adapters_common::workspace::{
    UNIVERSAL_WORKSPACE_STATE_NAMES, filesystem_workspace_entries, git_workspace_entries,
    has_git_ancestor,
};
use ayni_core::DependencyPreparationPlan;
use std::fs;
use std::io::{ErrorKind, Read, Write};
use std::path::Path;
use std::time::Duration;

const INPUT_LIMIT_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const ENTRY_LIMIT: usize = 500_000;
const GIT_TIMEOUT: Duration = Duration::from_secs(60);

pub(super) struct ManagedWorkspaceSnapshot {
    pub(super) checkout: tempfile::TempDir,
    pub(super) manifest: tempfile::NamedTempFile,
}

pub(super) fn path_is_excluded(relative: &str, prepared_outputs: &[String]) -> bool {
    relative
        .split('/')
        .any(|component| UNIVERSAL_WORKSPACE_STATE_NAMES.contains(&component))
        || prepared_outputs.iter().any(|output| {
            relative == output
                || relative
                    .strip_prefix(output)
                    .is_some_and(|suffix| suffix.starts_with('/'))
        })
}

fn prepared_outputs(
    preparations: &[DependencyPreparationPlan],
) -> Result<Vec<String>, BackendError> {
    let outputs = crate::preparation::unique_outputs(preparations)
        .into_iter()
        .map(|output| output.mount_path)
        .collect::<Vec<_>>();
    if outputs.iter().any(|path| path == ".") {
        return Err(BackendError::input(
            "managed dependency preparation cannot replace the repository root",
        ));
    }
    Ok(outputs)
}

fn workspace_entries(root: &Path) -> Result<Vec<std::path::PathBuf>, BackendError> {
    if has_git_ancestor(root) {
        git_workspace_entries(root, GIT_TIMEOUT).map_err(|error| {
            BackendError::input(format!(
                "failed to enumerate the managed Git workspace: {error}"
            ))
        })
    } else {
        filesystem_workspace_entries(root, ENTRY_LIMIT).map_err(|error| {
            BackendError::input(format!(
                "failed to enumerate the managed filesystem workspace: {error}"
            ))
        })
    }
}

#[cfg(unix)]
fn open_parent_directory(root: &Path, relative: &Path) -> std::io::Result<fs::File> {
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;

    use std::os::unix::fs::OpenOptionsExt;

    let mut directory = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(root)?;
    if let Some(parent) = relative.parent() {
        for component in parent.components() {
            let std::path::Component::Normal(name) = component else {
                return Err(std::io::Error::new(
                    ErrorKind::InvalidInput,
                    "non-normalized snapshot path",
                ));
            };
            let name = std::ffi::CString::new(name.as_bytes()).map_err(|_| {
                std::io::Error::new(ErrorKind::InvalidInput, "snapshot path contains NUL")
            })?;
            let descriptor = unsafe {
                libc::openat(
                    directory.as_raw_fd(),
                    name.as_ptr(),
                    libc::O_RDONLY | libc::O_CLOEXEC | libc::O_DIRECTORY | libc::O_NOFOLLOW,
                )
            };
            if descriptor < 0 {
                return Err(std::io::Error::last_os_error());
            }
            directory = unsafe { fs::File::from_raw_fd(descriptor) };
        }
    }
    Ok(directory)
}

#[cfg(unix)]
fn relative_file_name(relative: &Path) -> std::io::Result<std::ffi::CString> {
    use std::os::unix::ffi::OsStrExt;

    let name = relative
        .file_name()
        .ok_or_else(|| std::io::Error::new(ErrorKind::InvalidInput, "snapshot path has no file"))?;
    std::ffi::CString::new(name.as_bytes())
        .map_err(|_| std::io::Error::new(ErrorKind::InvalidInput, "snapshot path contains NUL"))
}

#[cfg(unix)]
fn open_regular_source(root: &Path, relative: &Path) -> std::io::Result<fs::File> {
    use std::os::fd::{AsRawFd, FromRawFd};

    let parent = open_parent_directory(root, relative)?;
    let name = relative_file_name(relative)?;
    let descriptor = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if descriptor < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(unsafe { fs::File::from_raw_fd(descriptor) })
    }
}

#[cfg(unix)]
fn read_symlink_target(root: &Path, relative: &Path) -> std::io::Result<std::path::PathBuf> {
    use std::os::fd::AsRawFd;
    use std::os::unix::ffi::OsStringExt;

    let parent = open_parent_directory(root, relative)?;
    let name = relative_file_name(relative)?;
    let mut capacity = 256usize;
    loop {
        let mut bytes = vec![0u8; capacity];
        let length = unsafe {
            libc::readlinkat(
                parent.as_raw_fd(),
                name.as_ptr(),
                bytes.as_mut_ptr().cast(),
                bytes.len(),
            )
        };
        if length < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let length = length as usize;
        if length < bytes.len() {
            bytes.truncate(length);
            return Ok(std::path::PathBuf::from(std::ffi::OsString::from_vec(
                bytes,
            )));
        }
        capacity = capacity.checked_mul(2).ok_or_else(|| {
            std::io::Error::new(ErrorKind::InvalidData, "symlink target is too long")
        })?;
    }
}

#[cfg(not(unix))]
fn validate_source_parent(root: &Path, relative: &Path) -> std::io::Result<()> {
    let parent = root.join(relative).parent().ok_or_else(|| {
        std::io::Error::new(ErrorKind::InvalidInput, "snapshot path has no parent")
    })?;
    if parent.canonicalize()?.starts_with(root) {
        Ok(())
    } else {
        Err(std::io::Error::new(
            ErrorKind::PermissionDenied,
            "snapshot path escapes its root",
        ))
    }
}

#[cfg(not(unix))]
fn open_regular_source(root: &Path, relative: &Path) -> std::io::Result<fs::File> {
    validate_source_parent(root, relative)?;
    fs::File::open(root.join(relative))
}

#[cfg(not(unix))]
fn read_symlink_target(root: &Path, relative: &Path) -> std::io::Result<std::path::PathBuf> {
    validate_source_parent(root, relative)?;
    fs::read_link(root.join(relative))
}

#[cfg(unix)]
fn create_symlink(target: &Path, destination: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, destination)
}

#[cfg(windows)]
fn create_symlink(target: &Path, destination: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_file(target, destination)
}

fn size_limit_error(limit: u64) -> BackendError {
    BackendError::input(format!(
        "managed workspace input exceeds the {limit} byte safety limit; ignore generated files or use --host"
    ))
}

fn create_parent(destination: &Path) -> Result<(), BackendError> {
    let Some(parent) = destination.parent() else {
        return Ok(());
    };
    fs::create_dir_all(parent).map_err(|error| {
        BackendError::execution(format!(
            "failed to create managed snapshot directory {}: {error}",
            parent.display()
        ))
    })
}

fn copy_symlink(
    root: &Path,
    relative_path: &Path,
    destination: &Path,
    relative: &str,
    total_bytes: &mut u64,
) -> Result<(), BackendError> {
    let target = read_symlink_target(root, relative_path).map_err(|error| {
        BackendError::execution(format!(
            "failed to read managed workspace symlink {relative}: {error}"
        ))
    })?;
    *total_bytes = total_bytes.saturating_add(target.as_os_str().len() as u64);
    if *total_bytes > INPUT_LIMIT_BYTES {
        return Err(size_limit_error(INPUT_LIMIT_BYTES));
    }
    create_symlink(&target, destination).map_err(|error| {
        BackendError::execution(format!(
            "failed to snapshot managed workspace symlink {relative}: {error}"
        ))
    })
}

fn copy_regular_file(
    root: &Path,
    relative_path: &Path,
    destination: &Path,
    relative: &str,
    total_bytes: &mut u64,
    limit: u64,
) -> Result<(), BackendError> {
    let input = open_regular_source(root, relative_path).map_err(|error| {
        BackendError::execution(format!(
            "failed to open managed workspace input {relative}: {error}"
        ))
    })?;
    let metadata = input.metadata().map_err(|error| {
        BackendError::execution(format!(
            "failed to inspect opened managed workspace input {relative}: {error}"
        ))
    })?;
    if !metadata.is_file() {
        return Err(BackendError::input(format!(
            "managed workspace input {relative} changed type while being snapshotted"
        )));
    }
    if total_bytes.saturating_add(metadata.len()) > limit {
        return Err(size_limit_error(limit));
    }
    let mut output = fs::File::create(destination).map_err(|error| {
        BackendError::execution(format!(
            "failed to create managed workspace snapshot file {relative}: {error}"
        ))
    })?;
    let remaining = limit - *total_bytes;
    let copied = std::io::copy(&mut input.take(remaining + 1), &mut output).map_err(|error| {
        BackendError::execution(format!(
            "failed to snapshot managed workspace input {relative}: {error}"
        ))
    })?;
    if copied > remaining {
        return Err(size_limit_error(limit));
    }
    *total_bytes += copied;
    fs::set_permissions(destination, metadata.permissions()).map_err(|error| {
        BackendError::execution(format!(
            "failed to preserve managed workspace permissions for {relative}: {error}"
        ))
    })
}

fn copy_entry(
    root: &Path,
    relative_path: &Path,
    destination: &Path,
    metadata: &fs::Metadata,
    relative: &str,
    total_bytes: &mut u64,
) -> Result<(), BackendError> {
    create_parent(destination)?;
    if metadata.file_type().is_symlink() {
        copy_symlink(root, relative_path, destination, relative, total_bytes)
    } else if metadata.is_file() {
        copy_regular_file(
            root,
            relative_path,
            destination,
            relative,
            total_bytes,
            INPUT_LIMIT_BYTES,
        )
    } else {
        Err(BackendError::input(format!(
            "managed workspace input {relative} is not a regular file or symlink"
        )))
    }
}

fn source_metadata(
    root: &Path,
    relative_path: &Path,
    relative: &str,
) -> Result<Option<fs::Metadata>, BackendError> {
    #[cfg(unix)]
    open_parent_directory(root, relative_path).map_err(|error| {
        BackendError::input(format!(
            "managed workspace input {relative} has an unsafe parent path: {error}"
        ))
    })?;
    match fs::symlink_metadata(root.join(relative_path)) {
        Ok(metadata) if metadata.is_dir() => Err(BackendError::input(format!(
            "managed workspace entry {relative} is a directory; initialize nested repositories separately"
        ))),
        Ok(metadata) => Ok(Some(metadata)),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(BackendError::execution(format!(
            "failed to inspect managed workspace input {relative}: {error}"
        ))),
    }
}

#[cfg(unix)]
fn ensure_secure_snapshot_platform() -> Result<(), BackendError> {
    Ok(())
}

#[cfg(not(unix))]
fn ensure_secure_snapshot_platform() -> Result<(), BackendError> {
    Err(BackendError::environment(
        "managed workspace snapshots currently require Unix no-follow filesystem primitives",
    ))
}

pub(super) fn create(
    root: &Path,
    preparations: &[DependencyPreparationPlan],
) -> Result<ManagedWorkspaceSnapshot, BackendError> {
    ensure_secure_snapshot_platform()?;
    let prepared_outputs = prepared_outputs(preparations)?;
    let entries = workspace_entries(root)?;
    let checkout = tempfile::TempDir::new().map_err(|error| {
        BackendError::execution(format!(
            "failed to create managed workspace snapshot: {error}"
        ))
    })?;
    let mut manifest = tempfile::NamedTempFile::new().map_err(|error| {
        BackendError::execution(format!(
            "failed to create managed workspace manifest: {error}"
        ))
    })?;
    let mut entry_count = 0usize;
    let mut total_bytes = 0u64;

    for relative_path in entries {
        let relative = relative_path
            .to_str()
            .ok_or_else(|| BackendError::input("managed workspace requires UTF-8 paths"))?;
        if path_is_excluded(relative, &prepared_outputs) {
            continue;
        }
        let Some(metadata) = source_metadata(root, &relative_path, relative)? else {
            continue;
        };
        entry_count += 1;
        if entry_count > ENTRY_LIMIT {
            return Err(BackendError::input(format!(
                "managed workspace input exceeds the {ENTRY_LIMIT} file safety limit; ignore generated files or use --host"
            )));
        }
        copy_entry(
            root,
            &relative_path,
            &checkout.path().join(&relative_path),
            &metadata,
            relative,
            &mut total_bytes,
        )?;
        manifest
            .write_all(relative.as_bytes())
            .and_then(|()| manifest.write_all(&[0]))
            .map_err(|error| {
                BackendError::execution(format!(
                    "failed to write managed workspace manifest: {error}"
                ))
            })?;
    }
    manifest.flush().map_err(|error| {
        BackendError::execution(format!(
            "failed to flush managed workspace manifest: {error}"
        ))
    })?;
    Ok(ManagedWorkspaceSnapshot { checkout, manifest })
}

#[cfg(test)]
mod tests {
    use super::copy_regular_file;
    use std::fs;

    #[test]
    fn regular_file_copy_enforces_the_byte_limit() {
        let root = tempfile::TempDir::new().expect("snapshot test");
        let source = root.path().join("source");
        let destination = root.path().join("destination");
        fs::write(&source, "four").expect("source");
        let mut total = 0;

        assert!(
            copy_regular_file(
                root.path(),
                std::path::Path::new("source"),
                &destination,
                "source",
                &mut total,
                3,
            )
            .is_err()
        );
        assert_eq!(total, 0);
        assert!(!destination.exists());
    }

    #[test]
    fn nested_git_workspace_uses_parent_ignore_rules() {
        let repository = tempfile::TempDir::new().expect("repository");
        let root = repository.path();
        let nested = root.join("packages/app");
        fs::create_dir_all(nested.join("target")).expect("nested generated directory");
        fs::write(root.join(".gitignore"), "target/\n").expect("ignore file");
        fs::write(nested.join("source.rs"), "source\n").expect("source");
        fs::write(nested.join("target/output"), "generated\n").expect("generated output");
        let git = |arguments: &[&str]| {
            let status = std::process::Command::new("git")
                .current_dir(root)
                .args(arguments)
                .status()
                .expect("git");
            assert!(status.success());
        };
        git(&["init", "-q"]);
        git(&["add", ".gitignore", "packages/app/source.rs"]);

        let snapshot = super::create(&nested, &[]).expect("nested snapshot");
        assert!(snapshot.checkout.path().join("source.rs").is_file());
        assert!(!snapshot.checkout.path().join("target").exists());
    }

    #[cfg(unix)]
    #[test]
    fn regular_file_copy_rejects_intermediate_symlink_escape() {
        use std::os::unix::fs::symlink;

        let root = tempfile::TempDir::new().expect("snapshot root");
        let outside = tempfile::TempDir::new().expect("outside root");
        fs::write(outside.path().join("secret"), "outside\n").expect("outside file");
        symlink(outside.path(), root.path().join("escape")).expect("escaping directory symlink");
        let destination = root.path().join("destination");
        let mut total = 0;

        assert!(
            copy_regular_file(
                root.path(),
                std::path::Path::new("escape/secret"),
                &destination,
                "escape/secret",
                &mut total,
                1024,
            )
            .is_err()
        );
        assert!(!destination.exists());
    }
}
