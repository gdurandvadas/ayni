use super::{Error, GIT_TIMEOUT, ensure_not_cancelled};
use ayni_adapters_common::exec::run_command_structured_cancellable;
use ayni_core::{CancellationToken, ChangeKind, ChangedPath};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

const UNTRACKED_HASH_CHUNK_SIZE: usize = 64 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct GitSnapshot {
    pub(super) requested_base: String,
    pub(super) base_commit: String,
    pub(super) head_commit: String,
    pub(super) fingerprint: String,
    pub(super) changes: Vec<ChangedPath>,
}

pub(super) fn git_snapshot(
    workspace_root: &Path,
    requested_base: &str,
    cancellation: &CancellationToken,
) -> Result<GitSnapshot, Error> {
    let context = resolve_git_context(workspace_root, requested_base, cancellation)?;
    reject_conflicts(&context.root, cancellation)?;
    // Git's combined diff represents the final local working-tree state: HEAD,
    // index, and worktree are deliberately not separate impact candidates.
    let tracked = workspace_git_bytes(
        &context,
        &[
            "diff",
            "--name-status",
            "-z",
            "--find-renames",
            "--find-copies",
            &context.base_commit,
        ],
        cancellation,
    )?;
    let untracked = nul_strings(&workspace_git_bytes(
        &context,
        &["ls-files", "--others", "--exclude-standard", "-z"],
        cancellation,
    )?)?;
    let changes = collect_workspace_changes(&tracked, &untracked, &context.prefix)?;
    let fingerprint = candidate_fingerprint(&context, &untracked, cancellation)?;
    Ok(GitSnapshot {
        requested_base: requested_base.to_owned(),
        base_commit: context.base_commit,
        head_commit: context.head_commit,
        fingerprint,
        changes,
    })
}

struct GitContext {
    root: PathBuf,
    prefix: String,
    base_commit: String,
    head_commit: String,
}

fn resolve_git_context(
    workspace_root: &Path,
    requested_base: &str,
    cancellation: &CancellationToken,
) -> Result<GitContext, Error> {
    let root = PathBuf::from(
        git_text(
            workspace_root,
            &["rev-parse", "--show-toplevel"],
            cancellation,
        )?
        .trim(),
    )
    .canonicalize()
    .map_err(|error| Error::input(format!("failed to resolve Git root: {error}")))?;
    let prefix = workspace_root
        .strip_prefix(&root)
        .map_err(|_| Error::input("configured repository root is outside the Git worktree"))?
        .to_string_lossy()
        .replace('\\', "/");
    let base_arg = format!("{requested_base}^{{commit}}");
    let base_commit = git_text(
        &root,
        &["rev-parse", "--verify", "--end-of-options", &base_arg],
        cancellation,
    )?
    .trim()
    .to_owned();
    let head_commit = git_text(
        &root,
        &["rev-parse", "--verify", "--end-of-options", "HEAD^{commit}"],
        cancellation,
    )?
    .trim()
    .to_owned();
    Ok(GitContext {
        root,
        prefix,
        base_commit,
        head_commit,
    })
}

fn reject_conflicts(git_root: &Path, cancellation: &CancellationToken) -> Result<(), Error> {
    let conflicts = git_bytes(
        git_root,
        &["diff", "--name-only", "--diff-filter=U", "-z", "--"],
        cancellation,
    )?;
    if conflicts.is_empty() {
        Ok(())
    } else {
        Err(Error::input(
            "impact planning requires a working tree without unresolved Git conflicts",
        ))
    }
}

fn collect_workspace_changes(
    tracked: &[u8],
    untracked: &[String],
    prefix: &str,
) -> Result<Vec<ChangedPath>, Error> {
    let mut changes = Vec::new();
    for change in parse_name_status(tracked)? {
        changes.extend(change_for_workspace(change, prefix));
    }
    for path in untracked {
        let normalized = normalize_git_path(path)?;
        if let Some(path) = path_for_workspace(&normalized, prefix) {
            changes.push(ChangedPath {
                kind: ChangeKind::Added,
                path,
                previous_path: None,
            });
        }
    }
    changes.sort();
    changes.dedup();
    Ok(changes)
}

fn workspace_git_bytes(
    context: &GitContext,
    args: &[&str],
    cancellation: &CancellationToken,
) -> Result<Vec<u8>, Error> {
    let mut args = args.iter().map(|arg| (*arg).to_owned()).collect::<Vec<_>>();
    args.push(String::from("--"));
    if !context.prefix.is_empty() {
        args.push(context.prefix.clone());
    }
    git_bytes(
        &context.root,
        &args.iter().map(String::as_str).collect::<Vec<_>>(),
        cancellation,
    )
}

fn candidate_fingerprint(
    context: &GitContext,
    untracked: &[String],
    cancellation: &CancellationToken,
) -> Result<String, Error> {
    let binary_diff = workspace_git_bytes(
        context,
        &["diff", "--binary", &context.base_commit],
        cancellation,
    )?;
    let mut hasher = Sha256::new();
    hash_segment(&mut hasher, b"base", context.base_commit.as_bytes());
    hash_segment(&mut hasher, b"head", context.head_commit.as_bytes());
    hash_segment(&mut hasher, b"tracked_diff", &binary_diff);
    for path in untracked {
        ensure_not_cancelled(cancellation, "impact fingerprinting")?;
        hash_untracked_path(&mut hasher, &context.root, path, cancellation)?;
    }
    Ok(format!("sha256:{:x}", hasher.finalize()))
}

pub(super) fn hash_untracked_path(
    hasher: &mut Sha256,
    git_root: &Path,
    path: &str,
    cancellation: &CancellationToken,
) -> Result<(), Error> {
    ensure_not_cancelled(cancellation, "impact fingerprinting")?;
    hash_segment(hasher, b"untracked_path", path.as_bytes());
    let candidate = git_root.join(path);
    let metadata = fs::symlink_metadata(&candidate).map_err(|error| {
        Error::input(format!("failed to inspect untracked path {path}: {error}"))
    })?;
    if metadata.file_type().is_symlink() {
        hash_untracked_symlink(hasher, &candidate, path, cancellation)?;
    } else if metadata.is_file() {
        hash_untracked_file(hasher, &candidate, path, cancellation)?;
    } else {
        return Err(Error::input(format!(
            "unsupported untracked filesystem entry {path}"
        )));
    }
    ensure_not_cancelled(cancellation, "impact fingerprinting")?;
    Ok(())
}

fn hash_untracked_symlink(
    hasher: &mut Sha256,
    candidate: &Path,
    path: &str,
    cancellation: &CancellationToken,
) -> Result<(), Error> {
    hash_segment(hasher, b"untracked_type", b"symlink");
    ensure_not_cancelled(cancellation, "impact fingerprinting")?;
    let target = fs::read_link(candidate).map_err(|error| {
        Error::input(format!("failed to read untracked symlink {path}: {error}"))
    })?;
    hash_segment(
        hasher,
        b"untracked_symlink_target",
        target.as_os_str().as_encoded_bytes(),
    );
    Ok(())
}

fn hash_untracked_file(
    hasher: &mut Sha256,
    candidate: &Path,
    path: &str,
    cancellation: &CancellationToken,
) -> Result<(), Error> {
    let mut file = fs::File::open(candidate)
        .map_err(|error| Error::input(format!("failed to open untracked file {path}: {error}")))?;
    let metadata = file.metadata().map_err(|error| {
        Error::input(format!("failed to inspect untracked file {path}: {error}"))
    })?;
    if !metadata.is_file() {
        return Err(untracked_file_changed(path));
    }
    hash_segment(hasher, b"untracked_type", b"file");
    hash_segment(
        hasher,
        b"untracked_executable",
        &[executable_bit(&metadata)],
    );
    hash_reader_segment(
        hasher,
        b"untracked_contents",
        &mut file,
        metadata.len(),
        cancellation,
        path,
    )
}

fn hash_reader_segment(
    hasher: &mut Sha256,
    label: &[u8],
    reader: &mut impl Read,
    expected_len: u64,
    cancellation: &CancellationToken,
    path: &str,
) -> Result<(), Error> {
    hash_segment_header(hasher, label, expected_len);
    let mut buffer = vec![0_u8; UNTRACKED_HASH_CHUNK_SIZE];
    let mut total = 0_u64;
    loop {
        ensure_not_cancelled(cancellation, "impact fingerprinting")?;
        let read = reader.read(&mut buffer).map_err(|error| {
            Error::input(format!("failed to read untracked file {path}: {error}"))
        })?;
        if read == 0 {
            break;
        }
        let next_total = total
            .checked_add(read as u64)
            .ok_or_else(|| untracked_file_changed(path))?;
        if next_total > expected_len {
            return Err(untracked_file_changed(path));
        }
        hasher.update(&buffer[..read]);
        total = next_total;
    }
    ensure_not_cancelled(cancellation, "impact fingerprinting")?;
    if total != expected_len {
        return Err(untracked_file_changed(path));
    }
    Ok(())
}

fn untracked_file_changed(path: &str) -> Error {
    Error::input(format!(
        "untracked file {path} changed while impact fingerprinting; rerun against a stable checkout"
    ))
}

fn hash_segment(hasher: &mut Sha256, label: &[u8], value: &[u8]) {
    hash_segment_header(hasher, label, value.len() as u64);
    hasher.update(value);
}

fn hash_segment_header(hasher: &mut Sha256, label: &[u8], value_len: u64) {
    hasher.update((label.len() as u64).to_be_bytes());
    hasher.update(label);
    hasher.update(value_len.to_be_bytes());
}

#[cfg(unix)]
fn executable_bit(metadata: &fs::Metadata) -> u8 {
    use std::os::unix::fs::PermissionsExt;
    u8::from(metadata.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn executable_bit(_metadata: &fs::Metadata) -> u8 {
    0
}

fn git_text(
    workdir: &Path,
    args: &[&str],
    cancellation: &CancellationToken,
) -> Result<String, Error> {
    let bytes = git_bytes(workdir, args, cancellation)?;
    String::from_utf8(bytes).map_err(|_| Error::input("Git returned a non-UTF-8 identity or path"))
}

fn git_bytes(
    workdir: &Path,
    args: &[&str],
    cancellation: &CancellationToken,
) -> Result<Vec<u8>, Error> {
    let args = args
        .iter()
        .map(|value| (*value).to_owned())
        .collect::<Vec<_>>();
    let output =
        run_command_structured_cancellable(workdir, "git", &args, GIT_TIMEOUT, cancellation)
            .map_err(|error| Error::execution(error.to_string()))?;
    if !output.status.success() {
        return Err(Error::input(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(output.stdout)
}

pub(super) fn parse_name_status(bytes: &[u8]) -> Result<Vec<ChangedPath>, Error> {
    let tokens = nul_strings(bytes)?;
    let mut index = 0;
    let mut changes = Vec::new();
    while let Some(token) = tokens.get(index) {
        index += 1;
        let (status, first_path) = status_and_path(token, &tokens, &mut index)?;
        changes.push(parse_status(status, first_path, &tokens, &mut index)?);
    }
    Ok(changes)
}

fn status_and_path(
    token: &str,
    tokens: &[String],
    index: &mut usize,
) -> Result<(char, String), Error> {
    let (status, inline_path) = token
        .split_once('\t')
        .map_or((token, None), |(status, path)| (status, Some(path)));
    let code = status
        .chars()
        .next()
        .ok_or_else(|| Error::input("Git emitted an empty change status"))?;
    let path = if let Some(path) = inline_path {
        path.to_owned()
    } else {
        let path = tokens
            .get(*index)
            .ok_or_else(|| Error::input("Git change status is missing a path"))?
            .clone();
        *index += 1;
        path
    };
    Ok((code, path))
}

fn parse_status(
    code: char,
    first_path: String,
    tokens: &[String],
    index: &mut usize,
) -> Result<ChangedPath, Error> {
    match code {
        'R' | 'C' => {
            let destination = tokens
                .get(*index)
                .ok_or_else(|| Error::input("Git rename/copy status is missing a destination"))?;
            *index += 1;
            Ok(ChangedPath {
                kind: if code == 'R' {
                    ChangeKind::Renamed
                } else {
                    ChangeKind::Copied
                },
                path: normalize_git_path(destination)?,
                previous_path: Some(normalize_git_path(&first_path)?),
            })
        }
        'A' | 'M' | 'D' | 'T' => Ok(ChangedPath {
            kind: simple_change_kind(code),
            path: normalize_git_path(&first_path)?,
            previous_path: None,
        }),
        other => Err(Error::input(format!(
            "unsupported Git change status {other:?}; resolve the worktree state first"
        ))),
    }
}

fn simple_change_kind(code: char) -> ChangeKind {
    match code {
        'A' => ChangeKind::Added,
        'D' => ChangeKind::Deleted,
        'T' => ChangeKind::TypeChanged,
        _ => ChangeKind::Modified,
    }
}

fn nul_strings(bytes: &[u8]) -> Result<Vec<String>, Error> {
    bytes
        .split(|byte| *byte == 0)
        .filter(|token| !token.is_empty())
        .map(|token| {
            String::from_utf8(token.to_vec())
                .map_err(|_| Error::input("Git returned a non-UTF-8 repository path"))
        })
        .collect()
}

fn normalize_git_path(path: &str) -> Result<String, Error> {
    if path.is_empty()
        || path.contains('\\')
        || path.starts_with('/')
        || path.split('/').any(|part| part == "..")
    {
        return Err(Error::input(format!(
            "Git returned an unsafe path: {path:?}"
        )));
    }
    Ok(path.to_owned())
}

fn path_for_workspace(path: &str, prefix: &str) -> Option<String> {
    if prefix.is_empty() {
        return Some(path.to_owned());
    }
    path.strip_prefix(prefix)
        .and_then(|path| path.strip_prefix('/'))
        .map(str::to_owned)
}

fn change_for_workspace(change: ChangedPath, prefix: &str) -> Vec<ChangedPath> {
    let current = path_for_workspace(&change.path, prefix);
    let previous = change
        .previous_path
        .as_deref()
        .and_then(|path| path_for_workspace(path, prefix));
    match (change.kind, current, previous) {
        (ChangeKind::Renamed | ChangeKind::Copied, Some(path), Some(previous_path)) => {
            vec![ChangedPath {
                kind: change.kind,
                path,
                previous_path: Some(previous_path),
            }]
        }
        (ChangeKind::Renamed, Some(path), None) | (ChangeKind::Copied, Some(path), None) => {
            vec![ChangedPath {
                kind: ChangeKind::Added,
                path,
                previous_path: None,
            }]
        }
        (ChangeKind::Renamed, None, Some(path)) => vec![ChangedPath {
            kind: ChangeKind::Deleted,
            path,
            previous_path: None,
        }],
        (_, Some(path), _) => vec![ChangedPath {
            kind: change.kind,
            path,
            previous_path: None,
        }],
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct RecordingReader {
        bytes: Vec<u8>,
        offset: usize,
        max_request: usize,
        read_calls: usize,
        cancel_after_first_read: Option<CancellationToken>,
    }

    impl RecordingReader {
        fn new(bytes: Vec<u8>) -> Self {
            Self {
                bytes,
                offset: 0,
                max_request: 0,
                read_calls: 0,
                cancel_after_first_read: None,
            }
        }
    }

    impl Read for RecordingReader {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            self.max_request = self.max_request.max(buffer.len());
            self.read_calls += 1;
            let remaining = &self.bytes[self.offset..];
            let read = remaining.len().min(buffer.len());
            buffer[..read].copy_from_slice(&remaining[..read]);
            self.offset += read;
            if self.read_calls == 1
                && let Some(cancellation) = &self.cancel_after_first_read
            {
                cancellation.cancel();
            }
            Ok(read)
        }
    }

    #[test]
    fn untracked_contents_are_hashed_with_bounded_reads_and_stable_framing() {
        let bytes = (0..UNTRACKED_HASH_CHUNK_SIZE * 2 + 17)
            .map(|index| (index % 251) as u8)
            .collect::<Vec<_>>();
        let mut reader = RecordingReader::new(bytes.clone());
        let cancellation = CancellationToken::default();
        let mut streamed = Sha256::new();

        hash_reader_segment(
            &mut streamed,
            b"untracked_contents",
            &mut reader,
            bytes.len() as u64,
            &cancellation,
            "large.bin",
        )
        .expect("streamed hash");

        let mut direct = Sha256::new();
        hash_segment(&mut direct, b"untracked_contents", &bytes);
        assert_eq!(streamed.finalize(), direct.finalize());
        assert_eq!(reader.max_request, UNTRACKED_HASH_CHUNK_SIZE);
        assert!(reader.read_calls >= 4, "multiple bounded reads plus EOF");
    }

    #[test]
    fn untracked_content_hashing_checks_cancellation_between_chunks() {
        let cancellation = CancellationToken::default();
        let mut reader = RecordingReader::new(vec![b'x'; UNTRACKED_HASH_CHUNK_SIZE * 2]);
        reader.cancel_after_first_read = Some(cancellation.clone());
        let mut hasher = Sha256::new();

        let error = hash_reader_segment(
            &mut hasher,
            b"untracked_contents",
            &mut reader,
            (UNTRACKED_HASH_CHUNK_SIZE * 2) as u64,
            &cancellation,
            "cancelled.bin",
        )
        .expect_err("cancelled hashing must stop");

        assert!(error.message.contains("aborted by Ctrl-C"));
        assert_eq!(reader.read_calls, 1);
    }
}
