use super::*;
use ayni_adapters_common::workspace::{
    git_workspace_entries, has_git_ancestor, is_universal_workspace_state,
};
use ayni_core::{lower_hex, sha256_fingerprint};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::time::Duration;

const MANAGED_LOCK_FINGERPRINT: &str = "AYNI_MANAGED_LOCK_FINGERPRINT";
const MANAGED_TOOL_VERSIONS: &str = "AYNI_MANAGED_TOOL_VERSIONS";

pub(crate) const SIGNALS_ARTIFACT: &str = ".ayni/last/signals.json";
pub(crate) const VERIFY_SIGNALS_ARTIFACT: &str = ".ayni/verify/last/signals.json";
pub(super) const ARTIFACTS_DIR: &str = ".ayni/last";

pub(super) fn build_artifact_metadata(
    config_path: &Path,
    workspace_root: &Path,
    planning: &AnalyzePlanning,
    output_mode: OutputArg,
) -> Result<RunArtifactMetadata, String> {
    build_artifact_metadata_for_command(config_path, workspace_root, planning, output_mode, "check")
}

pub(crate) fn build_artifact_metadata_for_command(
    config_path: &Path,
    workspace_root: &Path,
    planning: &AnalyzePlanning,
    output_mode: OutputArg,
    command: &str,
) -> Result<RunArtifactMetadata, String> {
    let generated_at = time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .map_err(|error| format!("failed to format analysis timestamp: {error}"))?;
    let languages = planning
        .targets
        .iter()
        .map(|target| target.language)
        .chain(planning.issues.iter().map(|issue| issue.language))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let scope = planning
        .targets
        .first()
        .map(|target| target.run_context.scope.clone());
    let managed = managed_execution_active();
    let mut tool_versions = if managed {
        let value = std::env::var(MANAGED_TOOL_VERSIONS)
            .map_err(|_| String::from("managed execution is missing tool-version provenance"))?;
        serde_json::from_str::<Vec<ArtifactToolVersion>>(&value)
            .map_err(|error| format!("managed tool-version provenance is invalid: {error}"))?
    } else {
        Vec::new()
    };
    tool_versions.sort();
    tool_versions.dedup();

    Ok(RunArtifactMetadata {
        generated_at,
        ayni_version: String::from(env!("CARGO_PKG_VERSION")),
        invocation: InvocationContext {
            command: command.to_string(),
            languages,
            scope,
        },
        output: OutputContext {
            format: output_mode.as_str().to_string(),
            destination: String::from("stdout"),
        },
        config_path: config_path.to_string_lossy().into_owned(),
        repository_root: workspace_root.to_string_lossy().into_owned(),
        execution_mode: if managed {
            ExecutionMode::Managed
        } else {
            ExecutionMode::Host
        },
        contract_digest: file_fingerprint(config_path)?,
        environment_lock_fingerprint: managed
            .then(|| std::env::var(MANAGED_LOCK_FINGERPRINT))
            .transpose()
            .map_err(|_| String::from("managed execution is missing lock provenance"))?,
        source_fingerprint: source_fingerprint(workspace_root)?,
        tool_versions,
    })
}

fn file_fingerprint(path: &Path) -> Result<String, String> {
    let bytes = fs::read(path)
        .map_err(|error| format!("failed to fingerprint {}: {error}", path.display()))?;
    Ok(sha256_fingerprint(bytes))
}

pub(crate) fn source_fingerprint(root: &Path) -> Result<String, String> {
    let files = if let Some(files) = managed_workspace_manifest_entries()? {
        files
    } else if has_git_ancestor(root) {
        collect_git_source_entries(root)?
    } else {
        let mut files = Vec::new();
        collect_source_entries(root, root, &mut files)?;
        files
    };
    fingerprint_source_entries(root, files)
}

pub(crate) fn source_fingerprint_excluding(
    root: &Path,
    excluded_outputs: &[String],
) -> Result<String, String> {
    let files = if has_git_ancestor(root) {
        collect_git_source_entries(root)?
    } else {
        let mut files = Vec::new();
        collect_source_entries(root, root, &mut files)?;
        files
    }
    .into_iter()
    .filter(|relative| {
        let relative = relative.to_string_lossy().replace('\\', "/");
        !excluded_outputs.iter().any(|output| {
            relative == *output
                || relative
                    .strip_prefix(output)
                    .is_some_and(|suffix| suffix.starts_with('/'))
        })
    })
    .collect();
    fingerprint_source_entries(root, files)
}

fn fingerprint_source_entries(root: &Path, mut files: Vec<PathBuf>) -> Result<String, String> {
    files.sort();
    files.dedup();
    let mut hasher = Sha256::new();
    for relative in files {
        let path = root.join(&relative);
        let relative = relative.to_string_lossy().replace('\\', "/");
        hash_field(&mut hasher, relative.as_bytes());
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                hasher.update(b"missing");
                continue;
            }
            Err(error) => {
                return Err(format!("failed to inspect {}: {error}", path.display()));
            }
        };
        if metadata.file_type().is_symlink() {
            hasher.update(b"symlink");
            let target = fs::read_link(&path)
                .map_err(|error| format!("failed to read symlink {}: {error}", path.display()))?;
            hash_os_str(&mut hasher, target.as_os_str());
        } else if metadata.is_file() {
            hasher.update(b"file");
            hasher.update([u8::from(is_executable(&metadata))]);
            hasher.update(metadata.len().to_be_bytes());
            let mut file = fs::File::open(&path)
                .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
            let mut buffer = [0u8; 64 * 1024];
            loop {
                let read = file
                    .read(&mut buffer)
                    .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
                if read == 0 {
                    break;
                }
                hasher.update(&buffer[..read]);
            }
        } else if metadata.is_dir() {
            hasher.update(b"directory");
        } else {
            hasher.update(b"special");
            hash_field(&mut hasher, special_file_type(&metadata).as_bytes());
        }
    }
    Ok(format!("sha256:{}", lower_hex(hasher.finalize())))
}

fn managed_workspace_manifest_entries() -> Result<Option<Vec<PathBuf>>, String> {
    if std::env::var_os("AYNI_MANAGED_LOCK_FINGERPRINT").is_none() {
        return Ok(None);
    }
    let Some(path) = std::env::var_os("AYNI_MANAGED_WORKSPACE_MANIFEST") else {
        return Ok(None);
    };
    let bytes = fs::read(&path).map_err(|error| {
        format!(
            "failed to read managed workspace manifest {}: {error}",
            Path::new(&path).display()
        )
    })?;
    let mut files = Vec::new();
    for raw in bytes
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
    {
        let relative = std::str::from_utf8(raw)
            .map_err(|_| String::from("managed workspace manifest requires UTF-8 paths"))?;
        let path = PathBuf::from(relative);
        if path.is_absolute()
            || path
                .components()
                .any(|component| !matches!(component, std::path::Component::Normal(_)))
        {
            return Err(String::from(
                "managed workspace manifest contains a non-normalized path",
            ));
        }
        files.push(path);
    }
    files.sort();
    files.dedup();
    Ok(Some(files))
}

fn collect_git_source_entries(root: &Path) -> Result<Vec<PathBuf>, String> {
    git_workspace_entries(root, Duration::from_secs(60))
        .map_err(|error| format!("failed to enumerate source fingerprint inputs: {error}"))
}

fn collect_source_entries(
    root: &Path,
    directory: &Path,
    files: &mut Vec<PathBuf>,
) -> Result<(), String> {
    let mut entries = fs::read_dir(directory)
        .map_err(|error| format!("failed to inspect {}: {error}", directory.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("failed to inspect {}: {error}", directory.display()))?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        let name = entry.file_name();
        if is_universal_workspace_state(&name.to_string_lossy()) {
            continue;
        }
        let file_type = entry
            .file_type()
            .map_err(|error| format!("failed to inspect {}: {error}", path.display()))?;
        files.push(
            path.strip_prefix(root)
                .map_err(|_| format!("source path {} escaped repository root", path.display()))?
                .to_path_buf(),
        );
        if file_type.is_dir() {
            collect_source_entries(root, &path, files)?;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn is_executable(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;

    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(_metadata: &fs::Metadata) -> bool {
    false
}

#[cfg(unix)]
fn special_file_type(metadata: &fs::Metadata) -> &'static str {
    use std::os::unix::fs::FileTypeExt;

    let file_type = metadata.file_type();
    if file_type.is_block_device() {
        "block"
    } else if file_type.is_char_device() {
        "char"
    } else if file_type.is_fifo() {
        "fifo"
    } else if file_type.is_socket() {
        "socket"
    } else {
        "unknown"
    }
}

#[cfg(not(unix))]
fn special_file_type(_metadata: &fs::Metadata) -> &'static str {
    "unknown"
}

#[cfg(unix)]
fn hash_os_str(hasher: &mut Sha256, value: &std::ffi::OsStr) {
    use std::os::unix::ffi::OsStrExt;

    hash_field(hasher, value.as_bytes());
}

#[cfg(windows)]
fn hash_os_str(hasher: &mut Sha256, value: &std::ffi::OsStr) {
    use std::os::windows::ffi::OsStrExt;

    let encoded = value
        .encode_wide()
        .flat_map(u16::to_le_bytes)
        .collect::<Vec<_>>();
    hash_field(hasher, &encoded);
}

#[cfg(not(any(unix, windows)))]
fn hash_os_str(hasher: &mut Sha256, value: &std::ffi::OsStr) {
    hash_field(hasher, value.to_string_lossy().as_bytes());
}

fn hash_field(hasher: &mut Sha256, value: &[u8]) {
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value);
}

pub(crate) fn serialize_artifact(artifact: &RunArtifact) -> Result<String, String> {
    serde_json::to_string_pretty(artifact)
        .map(|serialized| format!("{serialized}\n"))
        .map_err(|error| format!("failed to serialize artifact: {error}"))
}

/// Remove evidence that cannot describe the current invocation, such as after
/// contract validation fails before target planning. Absence is safer than a
/// prior successful artifact whose contract digest no longer matches.
pub(crate) fn invalidate_artifact_at(repo_root: &Path, relative_path: &str) -> Result<(), String> {
    let destination = repo_root.join(relative_path);
    match fs::remove_file(&destination) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "failed to invalidate stale artifact {relative_path}: {error}"
        )),
    }
}

pub(crate) fn persist_artifact_at(
    repo_root: &Path,
    relative_path: &str,
    serialized: &str,
) -> Result<(), String> {
    let destination = repo_root.join(relative_path);
    let parent = destination
        .parent()
        .ok_or_else(|| format!("artifact path {relative_path} has no parent"))?;
    fs::create_dir_all(parent).map_err(|error| {
        format!("failed to create artifact directory for {relative_path}: {error}")
    })?;
    let file_name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("artifact path {relative_path} has no file name"))?;
    let temporary = parent.join(format!(".{file_name}.tmp-{}", std::process::id()));
    fs::write(&temporary, serialized).map_err(|error| {
        format!("failed to write temporary artifact for {relative_path}: {error}")
    })?;
    if let Err(error) = fs::rename(&temporary, &destination) {
        let _ = fs::remove_file(&temporary);
        return Err(format!(
            "failed to atomically replace {relative_path}: {error}"
        ));
    }
    Ok(())
}

pub(crate) fn emit_analyze_outputs(
    output_mode: OutputArg,
    policy: &AyniPolicy,
    artifact: &RunArtifact,
    serialized: &str,
) -> Result<(), String> {
    match output_mode {
        OutputArg::Stdout => {
            ui::report::print_from_artifact(artifact, policy.report.offenders_limit);
        }
        OutputArg::Md => {
            ui::progress_log::log_command_failures(artifact);
            let summary = ui::md_report::build_markdown(artifact, policy.report.offenders_limit);
            println!("{summary}");
        }
        OutputArg::Json => {
            print!("{serialized}");
        }
    }
    Ok(())
}

pub(crate) fn workspace_root_from_config_path(config_path: &Path) -> Result<PathBuf, String> {
    let parent = config_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    parent.canonicalize().map_err(|error| {
        format!(
            "failed to resolve repository root {} for contract {}: {error}",
            parent.display(),
            config_path.display()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::{fingerprint_source_entries, source_fingerprint, workspace_root_from_config_path};
    use ayni_adapters_common::workspace::UNIVERSAL_WORKSPACE_STATE_NAMES;
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;

    #[test]
    fn default_relative_contract_resolves_a_canonical_operational_root() {
        let expected = std::env::current_dir()
            .expect("current directory")
            .canonicalize()
            .expect("canonical current directory");

        let actual =
            workspace_root_from_config_path(Path::new("./.ayni.toml")).expect("workspace root");

        assert_eq!(actual, expected);
        assert!(actual.is_absolute());
    }

    #[test]
    fn absolute_and_relative_contract_spellings_resolve_the_same_root() {
        let expected = std::env::current_dir()
            .expect("current directory")
            .canonicalize()
            .expect("canonical current directory");
        let absolute = expected.join(".ayni.toml");

        assert_eq!(
            workspace_root_from_config_path(Path::new("./.ayni.toml"))
                .expect("relative workspace root"),
            workspace_root_from_config_path(&absolute).expect("absolute workspace root")
        );
    }

    #[test]
    fn universal_workspace_state_does_not_change_source_provenance() {
        let directory = TempDir::new().expect("fixture");
        fs::write(directory.path().join("source.rs"), "fn main() {}\n").expect("source");
        let before = source_fingerprint(directory.path()).expect("before");

        for name in UNIVERSAL_WORKSPACE_STATE_NAMES
            .iter()
            .copied()
            .filter(|name| *name != ".git")
        {
            let state = directory.path().join(name);
            fs::create_dir(&state).expect("state directory");
            fs::write(state.join("output"), "state").expect("state output");
        }

        assert_eq!(source_fingerprint(directory.path()).expect("after"), before);
    }

    #[test]
    fn source_provenance_includes_tracked_source_and_ignores_generated_output() {
        let directory = TempDir::new().expect("fixture");
        let source = directory.path().join("src/build/checked_in.rs");
        let generated = directory.path().join("target/debug/output");
        fs::create_dir_all(source.parent().expect("source parent")).expect("source directory");
        fs::create_dir_all(generated.parent().expect("generated parent"))
            .expect("generated directory");
        fs::write(&source, "pub const VALUE: u8 = 1;\n").expect("source");
        fs::write(&generated, "first\n").expect("generated output");
        fs::write(directory.path().join(".gitignore"), "target/\n").expect("ignore file");
        let git = |arguments: &[&str]| {
            let status = std::process::Command::new("git")
                .current_dir(directory.path())
                .args(arguments)
                .status()
                .expect("git");
            assert!(status.success());
        };
        git(&["init", "-q"]);
        git(&["add", ".gitignore", "src/build/checked_in.rs"]);
        let before = source_fingerprint(directory.path()).expect("before");

        fs::write(&generated, "second\n").expect("changed generated output");
        assert_eq!(
            source_fingerprint(directory.path()).expect("after generated change"),
            before
        );

        let nested_root = directory.path().join("src");
        let nested_generated = nested_root.join("target/output");
        fs::create_dir_all(nested_generated.parent().expect("nested generated parent"))
            .expect("nested generated directory");
        fs::write(&nested_generated, "first\n").expect("nested generated output");
        let nested_before = source_fingerprint(&nested_root).expect("nested Git workspace");
        fs::write(&nested_generated, "second\n").expect("changed nested generated output");
        assert_eq!(
            source_fingerprint(&nested_root).expect("nested generated change"),
            nested_before
        );

        fs::write(&source, "pub const VALUE: u8 = 2;\n").expect("changed source");
        assert_ne!(source_fingerprint(directory.path()).expect("after"), before);
    }

    #[test]
    fn managed_manifest_fingerprint_ignores_new_output_but_detects_source_changes() {
        let directory = TempDir::new().expect("fixture");
        fs::write(directory.path().join("source.rs"), "first\n").expect("source");
        let entries = vec![std::path::PathBuf::from("source.rs")];
        let before = fingerprint_source_entries(directory.path(), entries.clone())
            .expect("manifest fingerprint");

        fs::create_dir(directory.path().join("coverage")).expect("generated directory");
        fs::write(directory.path().join("coverage/output.json"), "generated\n")
            .expect("generated output");
        assert_eq!(
            fingerprint_source_entries(directory.path(), entries.clone())
                .expect("unchanged manifest fingerprint"),
            before
        );

        fs::write(directory.path().join("source.rs"), "second\n").expect("changed source");
        assert_ne!(
            fingerprint_source_entries(directory.path(), entries)
                .expect("changed manifest fingerprint"),
            before
        );
    }

    #[cfg(unix)]
    #[test]
    fn source_provenance_distinguishes_executable_files_and_symlinks() {
        use std::os::unix::fs::{PermissionsExt, symlink};

        let directory = TempDir::new().expect("fixture");
        let entry = directory.path().join("entry");
        fs::write(&entry, "same bytes\n").expect("file");
        fs::set_permissions(&entry, fs::Permissions::from_mode(0o644)).expect("non-executable");
        let regular = source_fingerprint(directory.path()).expect("regular");

        fs::set_permissions(&entry, fs::Permissions::from_mode(0o755)).expect("executable");
        let executable = source_fingerprint(directory.path()).expect("executable fingerprint");
        assert_ne!(regular, executable);

        fs::remove_file(&entry).expect("remove file");
        symlink("target-a", &entry).expect("symlink");
        let symlink_a = source_fingerprint(directory.path()).expect("symlink fingerprint");
        assert_ne!(executable, symlink_a);

        fs::remove_file(&entry).expect("remove symlink");
        symlink("target-b", &entry).expect("second symlink");
        let symlink_b = source_fingerprint(directory.path()).expect("second symlink fingerprint");
        assert_ne!(symlink_a, symlink_b);

        fs::remove_file(&entry).expect("remove second symlink");
        symlink("/opt/ayni/prepared-source", &entry).expect("prepared-looking source symlink");
        let prepared_looking =
            source_fingerprint(directory.path()).expect("prepared-looking symlink fingerprint");
        assert_ne!(symlink_b, prepared_looking);

        use std::os::unix::ffi::OsStringExt;
        fs::remove_file(&entry).expect("remove prepared-looking symlink");
        symlink(std::ffi::OsString::from_vec(vec![0xff]), &entry)
            .expect("non-UTF-8 symlink target");
        let non_utf8_a = source_fingerprint(directory.path()).expect("non-UTF-8 target");
        fs::remove_file(&entry).expect("remove non-UTF-8 symlink");
        symlink(std::ffi::OsString::from_vec(vec![0xfe]), &entry)
            .expect("second non-UTF-8 symlink target");
        let non_utf8_b = source_fingerprint(directory.path()).expect("second non-UTF-8 target");
        assert_ne!(non_utf8_a, non_utf8_b);
    }
}
