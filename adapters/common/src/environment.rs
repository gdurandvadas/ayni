//! Reusable conformance checks for adapter environment capabilities.

use ayni_core::{
    AdapterError, EnvironmentCapability, EnvironmentContribution, EnvironmentDiscoveryRequest,
    Language,
};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

#[derive(Debug, Clone, PartialEq, Eq)]
struct SnapshotMetadata {
    len: u64,
    readonly: bool,
    modified: Option<(u64, u32)>,
    #[cfg(unix)]
    mode: u32,
    #[cfg(unix)]
    uid: u32,
    #[cfg(unix)]
    gid: u32,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    #[cfg(unix)]
    status_changed: (i64, i64),
}

impl SnapshotMetadata {
    fn from(metadata: &fs::Metadata) -> Self {
        let modified = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|duration| (duration.as_secs(), duration.subsec_nanos()));
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            Self {
                len: metadata.len(),
                readonly: metadata.permissions().readonly(),
                modified,
                mode: metadata.mode(),
                uid: metadata.uid(),
                gid: metadata.gid(),
                device: metadata.dev(),
                inode: metadata.ino(),
                status_changed: (metadata.ctime(), metadata.ctime_nsec()),
            }
        }
        #[cfg(not(unix))]
        {
            Self {
                len: metadata.len(),
                readonly: metadata.permissions().readonly(),
                modified,
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SnapshotEntry {
    Directory(SnapshotMetadata),
    File {
        metadata: SnapshotMetadata,
        content: Vec<u8>,
    },
    Symlink {
        metadata: SnapshotMetadata,
        target: PathBuf,
    },
}

/// Run shared environment-capability conformance checks.
///
/// The harness calls discovery twice, proves deterministic typed output, checks
/// target/language identity, and verifies that repository files were not
/// mutated. Callers should prepare a bounded fixture repository.
pub fn assert_environment_capability_conformance(
    capability: &dyn EnvironmentCapability,
    request: &EnvironmentDiscoveryRequest,
) -> Result<EnvironmentContribution, AdapterError> {
    validate_request_language(capability, request)?;
    let before = snapshot(request.repo_root()).map_err(|error| {
        AdapterError::new(
            capability.language(),
            format!("failed to snapshot environment fixture: {error}"),
        )
    })?;
    let first = discover_without_mutation(capability, request, &before)?;
    let first_json = serialize_contribution(capability.language(), &first)?;
    let second = discover_without_mutation(capability, request, &before)?;
    let second_json = serialize_contribution(capability.language(), &second)?;
    if first_json != second_json {
        return Err(AdapterError::new(
            capability.language(),
            "environment discovery is not deterministic",
        ));
    }
    validate_contribution(capability.language(), request, &first)?;
    Ok(first)
}

fn discover_without_mutation(
    capability: &dyn EnvironmentCapability,
    request: &EnvironmentDiscoveryRequest,
    before: &BTreeMap<PathBuf, SnapshotEntry>,
) -> Result<EnvironmentContribution, AdapterError> {
    let result = capability.discover(request);
    let after = snapshot(request.repo_root()).map_err(|error| {
        AdapterError::new(
            capability.language(),
            format!("failed to snapshot environment fixture after discovery: {error}"),
        )
    })?;
    if before != &after {
        return Err(AdapterError::new(
            capability.language(),
            "environment discovery mutated the repository",
        ));
    }
    result
}

fn serialize_contribution(
    language: Language,
    contribution: &EnvironmentContribution,
) -> Result<Vec<u8>, AdapterError> {
    serde_json::to_vec(contribution).map_err(|error| {
        AdapterError::new(
            language,
            format!("failed to serialize environment contribution: {error}"),
        )
    })
}

fn validate_request_language(
    capability: &dyn EnvironmentCapability,
    request: &EnvironmentDiscoveryRequest,
) -> Result<(), AdapterError> {
    if capability.language() == request.target().language {
        Ok(())
    } else {
        Err(AdapterError::new(
            capability.language(),
            "environment request language does not match capability language",
        ))
    }
}

fn validate_contribution(
    language: Language,
    request: &EnvironmentDiscoveryRequest,
    contribution: &EnvironmentContribution,
) -> Result<(), AdapterError> {
    if contribution.target().target == *request.target() {
        Ok(())
    } else {
        Err(AdapterError::new(
            language,
            "environment contribution target does not match the requested target",
        ))
    }
}

fn snapshot(root: &Path) -> Result<BTreeMap<PathBuf, SnapshotEntry>, String> {
    let metadata = fs::symlink_metadata(root)
        .map_err(|error| format!("failed to inspect {}: {error}", root.display()))?;
    if !metadata.is_dir() {
        return Err(format!(
            "snapshot root is not a directory: {}",
            root.display()
        ));
    }
    let mut snapshot = BTreeMap::from([(
        PathBuf::from("."),
        SnapshotEntry::Directory(SnapshotMetadata::from(&metadata)),
    )]);
    visit(root, root, &mut snapshot)?;
    Ok(snapshot)
}

fn visit(
    root: &Path,
    current: &Path,
    snapshot: &mut BTreeMap<PathBuf, SnapshotEntry>,
) -> Result<(), String> {
    let mut entries = fs::read_dir(current)
        .map_err(|error| format!("failed to read {}: {error}", current.display()))?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("failed to read {} entry: {error}", current.display()))?;
    entries.sort();
    for path in entries {
        let relative = path
            .strip_prefix(root)
            .map_err(|error| format!("failed to relativize {}: {error}", path.display()))?
            .to_path_buf();
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| format!("failed to inspect {}: {error}", path.display()))?;
        let snapshot_metadata = SnapshotMetadata::from(&metadata);
        if metadata.file_type().is_symlink() {
            let target = fs::read_link(&path)
                .map_err(|error| format!("failed to read symlink {}: {error}", path.display()))?;
            snapshot.insert(
                relative,
                SnapshotEntry::Symlink {
                    metadata: snapshot_metadata,
                    target,
                },
            );
        } else if metadata.is_dir() {
            snapshot.insert(relative, SnapshotEntry::Directory(snapshot_metadata));
            visit(root, &path, snapshot)?;
        } else {
            let content = fs::read(&path)
                .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
            snapshot.insert(
                relative,
                SnapshotEntry::File {
                    metadata: snapshot_metadata,
                    content,
                },
            );
        }
    }
    Ok(())
}

/// Run shared conformance checks for a dependency-preparation capability.
/// Planning must be deterministic and read-only; commands are validated by the
/// core contract and are deliberately not executed by this harness.
pub fn assert_dependency_preparation_conformance(
    capability: &dyn ayni_core::DependencyPreparationCapability,
    request: &ayni_core::DependencyPreparationRequest,
) -> Result<ayni_core::DependencyPreparationPlan, AdapterError> {
    if capability.language() != request.target().target.language {
        return Err(AdapterError::new(
            capability.language(),
            "dependency preparation request language does not match capability language",
        ));
    }
    let before = snapshot(request.repo_root()).map_err(|error| {
        AdapterError::new(
            capability.language(),
            format!("failed to snapshot preparation fixture: {error}"),
        )
    })?;
    let first = capability.prepare(request)?;
    let after = snapshot(request.repo_root()).map_err(|error| {
        AdapterError::new(
            capability.language(),
            format!("failed to snapshot preparation fixture after planning: {error}"),
        )
    })?;
    if before != after {
        return Err(AdapterError::new(
            capability.language(),
            "dependency preparation planning mutated the repository",
        ));
    }
    let second = capability.prepare(request)?;
    if first != second {
        return Err(AdapterError::new(
            capability.language(),
            "dependency preparation planning is not deterministic",
        ));
    }
    if first.target != request.target().target {
        return Err(AdapterError::new(
            capability.language(),
            "dependency preparation target does not match the requested target",
        ));
    }
    Ok(first)
}

#[cfg(test)]
mod tests {
    use super::assert_environment_capability_conformance;
    use ayni_core::{
        AdapterError, Architecture, EnvironmentCapability, EnvironmentContribution,
        EnvironmentDiscoveryRequest, Language, Libc, OperatingSystem, RequirementConfidence,
        RequirementSource, RuntimeRequirement, TargetEnvironment, TargetIdentity, TargetPlatform,
        VersionRequirement,
    };
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tempfile::TempDir;

    struct FixtureCapability {
        language: Language,
        mode: Mode,
        calls: AtomicUsize,
    }

    enum Mode {
        Valid,
        WrongTarget,
        Nondeterministic,
        Mutating,
        MutatingError,
        MutatingMetadata,
    }

    impl EnvironmentCapability for FixtureCapability {
        fn language(&self) -> Language {
            self.language
        }

        fn discover(
            &self,
            request: &EnvironmentDiscoveryRequest,
        ) -> Result<EnvironmentContribution, AdapterError> {
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            if matches!(self.mode, Mode::Mutating | Mode::MutatingError) {
                fs::write(request.repo_root().join("mutated"), "changed")
                    .map_err(|error| AdapterError::new(self.language, error.to_string()))?;
            }
            if matches!(self.mode, Mode::MutatingMetadata) {
                let manifest = request.repo_root().join("manifest.toml");
                let mut permissions = fs::metadata(&manifest)
                    .map_err(|error| AdapterError::new(self.language, error.to_string()))?
                    .permissions();
                permissions.set_readonly(!permissions.readonly());
                fs::set_permissions(&manifest, permissions)
                    .map_err(|error| AdapterError::new(self.language, error.to_string()))?;
            }
            if matches!(self.mode, Mode::MutatingError) {
                return Err(AdapterError::new(self.language, "discovery failed"));
            }
            let target = if matches!(self.mode, Mode::WrongTarget) {
                TargetIdentity::new(self.language, "other").expect("target")
            } else {
                request.target().clone()
            };
            let runtime = if matches!(self.mode, Mode::Nondeterministic) && call > 0 {
                "runtime-b"
            } else {
                "runtime-a"
            };
            EnvironmentContribution::new(
                TargetEnvironment {
                    target,
                    workspace: None,
                    package: None,
                    runtimes: vec![RuntimeRequirement {
                        runtime: String::from(runtime),
                        version: VersionRequirement::exact("1.0.0").expect("version"),
                        components: Vec::new(),
                        targets: Vec::new(),
                        source: RequirementSource::new(
                            "manifest",
                            "manifest.toml",
                            None::<String>,
                            RequirementConfidence::Declared,
                        )
                        .expect("source"),
                    }],
                    package_manager: None,
                    signal_tools: Vec::new(),
                    system_requirements: Vec::new(),
                    dependency_locks: Vec::new(),
                },
                Vec::new(),
                Vec::new(),
            )
            .map_err(|error| AdapterError::new(self.language, error.to_string()))
        }
    }

    fn request(root: &std::path::Path) -> EnvironmentDiscoveryRequest {
        EnvironmentDiscoveryRequest::new(
            root.to_path_buf(),
            TargetIdentity::new(Language::Rust, ".").expect("target"),
            [],
            vec![TargetPlatform {
                os: OperatingSystem::Linux,
                architecture: Architecture::Amd64,
                libc: Libc::Glibc,
            }],
        )
        .expect("request")
    }

    fn capability(mode: Mode) -> FixtureCapability {
        FixtureCapability {
            language: Language::Rust,
            mode,
            calls: AtomicUsize::new(0),
        }
    }

    #[test]
    fn valid_capability_is_deterministic_and_read_only() {
        let fixture = TempDir::new().expect("tempdir");
        fs::write(fixture.path().join("manifest.toml"), "runtime=1").expect("fixture manifest");
        assert_environment_capability_conformance(
            &capability(Mode::Valid),
            &request(fixture.path()),
        )
        .expect("conformance");
        assert!(!fixture.path().join("mutated").exists());
    }

    #[test]
    fn harness_rejects_wrong_target_and_nondeterminism() {
        for (mode, expected) in [
            (Mode::WrongTarget, "does not match"),
            (Mode::Nondeterministic, "not deterministic"),
        ] {
            let fixture = TempDir::new().expect("tempdir");
            fs::write(fixture.path().join("manifest.toml"), "runtime=1").expect("fixture manifest");
            let error = assert_environment_capability_conformance(
                &capability(mode),
                &request(fixture.path()),
            )
            .expect_err("must fail");
            assert!(error.message.contains(expected), "{}", error.message);
        }
    }

    #[test]
    fn harness_rejects_repository_mutation_on_success_or_error() {
        for mode in [Mode::Mutating, Mode::MutatingError, Mode::MutatingMetadata] {
            let fixture = TempDir::new().expect("tempdir");
            fs::write(fixture.path().join("manifest.toml"), "runtime=1").expect("fixture manifest");
            let error = assert_environment_capability_conformance(
                &capability(mode),
                &request(fixture.path()),
            )
            .expect_err("must fail");
            assert!(error.message.contains("mutated the repository"));
        }
    }
}
