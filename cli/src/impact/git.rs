use super::{Error, GIT_TIMEOUT};
use ayni_adapters_common::exec::run_command_structured;
use ayni_core::{ChangeKind, ChangedPath};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

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
) -> Result<GitSnapshot, Error> {
    let context = resolve_git_context(workspace_root, requested_base)?;
    reject_conflicts(&context.root)?;
    let tracked = git_bytes(
        &context.root,
        &[
            "diff",
            "--name-status",
            "-z",
            "--find-renames",
            &context.base_commit,
            "--",
        ],
    )?;
    let untracked = nul_strings(&git_bytes(
        &context.root,
        &["ls-files", "--others", "--exclude-standard", "-z"],
    )?)?;
    let changes = collect_workspace_changes(&tracked, &untracked, &context.prefix)?;
    let fingerprint = candidate_fingerprint(&context, &untracked)?;
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

fn resolve_git_context(workspace_root: &Path, requested_base: &str) -> Result<GitContext, Error> {
    let root = PathBuf::from(git_text(workspace_root, &["rev-parse", "--show-toplevel"])?.trim())
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
    )?
    .trim()
    .to_owned();
    let head_commit = git_text(
        &root,
        &["rev-parse", "--verify", "--end-of-options", "HEAD^{commit}"],
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

fn reject_conflicts(git_root: &Path) -> Result<(), Error> {
    let conflicts = git_bytes(
        git_root,
        &["diff", "--name-only", "--diff-filter=U", "-z", "--"],
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

fn candidate_fingerprint(context: &GitContext, untracked: &[String]) -> Result<String, Error> {
    let binary_diff = git_bytes(
        &context.root,
        &["diff", "--binary", &context.base_commit, "--"],
    )?;
    let mut hasher = Sha256::new();
    hash_segment(&mut hasher, b"base", context.base_commit.as_bytes());
    hash_segment(&mut hasher, b"head", context.head_commit.as_bytes());
    hash_segment(&mut hasher, b"tracked_diff", &binary_diff);
    for path in untracked {
        hash_untracked_path(&mut hasher, &context.root, path)?;
    }
    Ok(format!("sha256:{:x}", hasher.finalize()))
}

pub(super) fn hash_untracked_path(
    hasher: &mut Sha256,
    git_root: &Path,
    path: &str,
) -> Result<(), Error> {
    hash_segment(hasher, b"untracked_path", path.as_bytes());
    let candidate = git_root.join(path);
    let metadata = fs::symlink_metadata(&candidate).map_err(|error| {
        Error::input(format!("failed to inspect untracked path {path}: {error}"))
    })?;
    if metadata.file_type().is_symlink() {
        hash_segment(hasher, b"untracked_type", b"symlink");
        let target = fs::read_link(&candidate).map_err(|error| {
            Error::input(format!("failed to read untracked symlink {path}: {error}"))
        })?;
        hash_segment(
            hasher,
            b"untracked_symlink_target",
            target.as_os_str().as_encoded_bytes(),
        );
    } else if metadata.is_file() {
        hash_segment(hasher, b"untracked_type", b"file");
        hash_segment(
            hasher,
            b"untracked_executable",
            &[executable_bit(&metadata)],
        );
        let contents = fs::read(&candidate).map_err(|error| {
            Error::input(format!("failed to read untracked file {path}: {error}"))
        })?;
        hash_segment(hasher, b"untracked_contents", &contents);
    } else {
        return Err(Error::input(format!(
            "unsupported untracked filesystem entry {path}"
        )));
    }
    Ok(())
}

fn hash_segment(hasher: &mut Sha256, label: &[u8], value: &[u8]) {
    hasher.update((label.len() as u64).to_be_bytes());
    hasher.update(label);
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value);
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

fn git_text(workdir: &Path, args: &[&str]) -> Result<String, Error> {
    let bytes = git_bytes(workdir, args)?;
    String::from_utf8(bytes).map_err(|_| Error::input("Git returned a non-UTF-8 identity or path"))
}

fn git_bytes(workdir: &Path, args: &[&str]) -> Result<Vec<u8>, Error> {
    let args = args
        .iter()
        .map(|value| (*value).to_owned())
        .collect::<Vec<_>>();
    let output = run_command_structured(workdir, "git", &args, GIT_TIMEOUT)
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
