use super::*;
use ayni_adapters_common::workspace::is_generated_workspace_entry;
use sha2::{Digest, Sha256};

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
    Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
}

fn source_fingerprint(root: &Path) -> Result<String, String> {
    let mut files = Vec::new();
    collect_source_entries(root, root, &mut files)?;
    files.sort();
    let mut hasher = Sha256::new();
    for relative in files {
        let path = root.join(&relative);
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| format!("failed to inspect {}: {error}", path.display()))?;
        let relative = relative.to_string_lossy().replace('\\', "/");
        hash_field(&mut hasher, relative.as_bytes());
        if metadata.file_type().is_symlink() {
            hasher.update(b"symlink");
            let target = fs::read_link(&path)
                .map_err(|error| format!("failed to read symlink {}: {error}", path.display()))?;
            hash_field(&mut hasher, target.to_string_lossy().as_bytes());
        } else if metadata.is_file() {
            hasher.update(b"file");
            hasher.update([u8::from(is_executable(&metadata))]);
            let bytes = fs::read(&path)
                .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
            hash_field(&mut hasher, &bytes);
        } else if metadata.is_dir() {
            hasher.update(b"directory");
        } else {
            hasher.update(b"special");
            hash_field(&mut hasher, special_file_type(&metadata).as_bytes());
        }
    }
    Ok(format!("sha256:{:x}", hasher.finalize()))
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
        if is_generated_workspace_entry(&name.to_string_lossy()) {
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

fn hash_field(hasher: &mut Sha256, value: &[u8]) {
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value);
}

pub(crate) fn serialize_artifact(artifact: &RunArtifact) -> Result<String, String> {
    serde_json::to_string_pretty(artifact)
        .map(|serialized| format!("{serialized}\n"))
        .map_err(|error| format!("failed to serialize artifact: {error}"))
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
    use super::{source_fingerprint, workspace_root_from_config_path};
    use ayni_adapters_common::workspace::GENERATED_WORKSPACE_ENTRY_NAMES;
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
    fn generated_workspace_entries_do_not_change_source_provenance() {
        let directory = TempDir::new().expect("fixture");
        fs::write(directory.path().join("source.rs"), "fn main() {}\n").expect("source");
        let before = source_fingerprint(directory.path()).expect("before");

        for name in GENERATED_WORKSPACE_ENTRY_NAMES {
            let generated = directory.path().join(name);
            fs::create_dir(&generated).expect("generated directory");
            fs::write(generated.join("output"), "generated").expect("generated output");
        }

        assert_eq!(source_fingerprint(directory.path()).expect("after"), before);
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
    }
}
