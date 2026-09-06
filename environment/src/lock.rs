use crate::{BackendError, concise_output};
use ayni_adapters_common::exec::run_command;
use ayni_core::{
    Architecture, EnvironmentLock, EnvironmentPlan, ProvisioningBase, sha256_fingerprint,
};
use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::path::Path;
use std::time::Duration;

pub const LOCK_FILE: &str = ".ayni.lock";
pub const BASE_VARIANT: &str = "debian";
pub const BASE_MISE_VERSION: &str = "2025.2.4";
const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);

/// Read the pinned durable substrate definition. An explicit
/// value must use `<reference>@sha256:<digest>`.
pub fn resolve_provisioning_base(
    _ayni_version: &str,
    explicit: Option<&str>,
) -> Result<ProvisioningBase, BackendError> {
    let mut base: ProvisioningBase = serde_json::from_str(include_str!("../provisioning.json"))
        .map_err(|error| {
            BackendError::input(format!("invalid built-in provisioning definition: {error}"))
        })?;
    if let Some(explicit) = explicit {
        let (reference, digest) = parse_exact_base(explicit)?;
        base.reference = reference;
        base.digest = digest;
    }
    Ok(base)
}

pub(crate) fn parse_exact_base(value: &str) -> Result<(String, String), BackendError> {
    let (reference, digest) = value.rsplit_once('@').ok_or_else(|| {
        BackendError::input("image reference must use <reference>@sha256:<digest>")
    })?;
    validate_reference(reference)?;
    validate_digest(digest)?;
    Ok((reference.to_owned(), digest.to_ascii_lowercase()))
}

pub(crate) fn inspect_remote_digest(reference: &str) -> Result<String, BackendError> {
    validate_reference(reference)?;
    let cwd = env::current_dir().map_err(|error| {
        BackendError::execution(format!("failed to establish current directory: {error}"))
    })?;
    let args = vec![
        "buildx".to_owned(),
        "imagetools".to_owned(),
        "inspect".to_owned(),
        reference.to_owned(),
        "--format".to_owned(),
        "{{json .Manifest}}".to_owned(),
    ];
    let output = run_command(&cwd, "docker", &args, COMMAND_TIMEOUT).map_err(|error| {
        BackendError::environment(format!(
            "failed to resolve immutable executor image {reference}: {error}; install Docker Buildx or pass `--executor-image <reference>@sha256:<digest>`"
        ))
    })?;
    if !output.status.success() {
        return Err(BackendError::environment(format!(
            "failed to resolve immutable executor image {reference}: {}; pass an available exact image with `--executor-image`",
            concise_output(&output.stderr)
        )));
    }
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).map_err(|error| {
        BackendError::environment(format!(
            "Docker returned malformed manifest metadata for {reference}: {error}"
        ))
    })?;
    let digest = value
        .get("digest")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            BackendError::environment(format!(
                "Docker returned no manifest digest for executor image {reference}"
            ))
        })?;
    validate_digest(digest)?;
    Ok(digest.to_ascii_lowercase())
}

/// Compare current adapter discovery with a validated lock without requiring
/// unresolved selectors to equal their locked exact versions.
pub fn plan_matches_lock(plan: &EnvironmentPlan, lock: &EnvironmentLock) -> bool {
    plan.repository().contract_digest == lock.repository().contract_digest
        && plan.capabilities() == lock.capabilities()
        && plan.resource_limits() == lock.resource_limits()
        && plan.targets().len() == lock.targets().len()
        && plan.targets().iter().all(|plan| {
            let Some(locked) = lock
                .targets()
                .iter()
                .find(|locked| locked.target == plan.target)
            else {
                return false;
            };
            plan.runtimes.len() == locked.runtimes.len()
                && plan
                    .runtimes
                    .iter()
                    .zip(&locked.runtimes)
                    .all(|(left, right)| {
                        left.runtime == right.runtime
                            && left.components == right.components
                            && left.targets == right.targets
                            && left.source.path == right.source.path
                    })
                && match (&plan.package_manager, &locked.package_manager) {
                    (None, None) => true,
                    (Some(left), Some(right)) => {
                        left.family == right.family
                            && left.ownership_root == right.ownership_root
                            && left.source.path == right.source.path
                    }
                    _ => false,
                }
                && plan.signal_tools.len() == locked.signal_tools.len()
                && plan
                    .signal_tools
                    .iter()
                    .zip(&locked.signal_tools)
                    .all(|(left, right)| signal_tool_matches_lock(left, right))
                && plan.dependency_locks.len() == locked.dependency_locks.len()
                && plan.dependency_locks.iter().all(|left| {
                    locked.dependency_locks.iter().any(|right| {
                        left.path == right.path
                            && left.digest == right.digest
                            && left.owner_root == right.owner_root
                    })
                })
        })
}

fn signal_tool_matches_lock(
    planned: &ayni_core::SignalToolRequirement,
    locked: &ayni_core::LockedSignalTool,
) -> bool {
    // Resolution may replace manifest evidence with native-lock evidence.
    // Exact baselines must still invalidate locks after an adapter upgrade.
    planned.tool == locked.tool
        && planned.provider == locked.provider
        && planned.scope == locked.scope
        && planned.version_authority == locked.version_authority
        && planned.signals == locked.signals
        && match &planned.version {
            ayni_core::VersionRequirement::Exact { version } => version == &locked.version,
            _ => true,
        }
}

pub fn read_lock(repo_root: &Path) -> Result<EnvironmentLock, BackendError> {
    let path = repo_root.join(LOCK_FILE);
    let metadata = fs::symlink_metadata(&path).map_err(|error| {
        BackendError::environment(format!(
            "environment lock {} is required: {error}; run `ayni env lock` and `ayni env build`",
            path.display()
        ))
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(BackendError::environment(format!(
            "environment lock must be a regular file: {}",
            path.display()
        )));
    }
    let bytes = fs::read(&path).map_err(|error| {
        BackendError::environment(format!(
            "failed to read environment lock {}: {error}",
            path.display()
        ))
    })?;
    let lock: EnvironmentLock = serde_json::from_slice(&bytes).map_err(|error| {
        BackendError::environment(format!(
            "environment lock {} is invalid: {error}; run `ayni env lock` and `ayni env build`",
            path.display()
        ))
    })?;
    validate_lock(repo_root, &lock)?;
    Ok(lock)
}

fn validate_lock(repo_root: &Path, lock: &EnvironmentLock) -> Result<(), BackendError> {
    let contract_path = &lock.repository().contract_path;
    let contract_digest = digest_contained_file(repo_root, &repo_root.join(contract_path))?;
    if contract_digest != lock.repository().contract_digest {
        return Err(BackendError::environment(format!(
            "environment lock is stale because {contract_path} changed; run `ayni env lock`"
        )));
    }
    let mut checked = BTreeSet::new();
    for target in lock.targets() {
        for source in target
            .runtimes
            .iter()
            .map(|item| &item.source)
            .chain(target.package_manager.iter().map(|item| &item.source))
            .chain(target.signal_tools.iter().map(|item| &item.source))
            .chain(target.dependency_locks.iter().map(|item| &item.source))
        {
            let Some(expected) = &source.digest else {
                continue;
            };
            if checked.insert(source.path.clone()) {
                ensure_digest(repo_root, &source.path, expected)?;
            }
        }
        for dependency in &target.dependency_locks {
            if checked.insert(dependency.path.clone()) {
                ensure_digest(repo_root, &dependency.path, &dependency.digest)?;
            }
        }
    }
    let host_architecture = host_architecture()?;
    if !lock
        .platforms()
        .iter()
        .any(|platform| platform.architecture == host_architecture)
    {
        return Err(BackendError::environment(format!(
            "environment lock does not support the host architecture {}",
            platform_architecture(host_architecture)
        )));
    }
    Ok(())
}

fn ensure_digest(repo_root: &Path, relative: &str, expected: &str) -> Result<(), BackendError> {
    let actual = digest_contained_file(repo_root, &repo_root.join(relative))?;
    if actual == expected {
        Ok(())
    } else {
        Err(BackendError::environment(format!(
            "environment lock is stale because {relative} changed; run `ayni env lock`"
        )))
    }
}

fn digest_contained_file(repo_root: &Path, path: &Path) -> Result<String, BackendError> {
    let canonical = path.canonicalize().map_err(|error| {
        BackendError::environment(format!(
            "failed to inspect locked environment input {}: {error}; run `ayni env lock` and `ayni env build`",
            path.display()
        ))
    })?;
    if !canonical.starts_with(repo_root) || !canonical.is_file() {
        return Err(BackendError::environment(format!(
            "locked environment input escapes the repository or is not a file: {}",
            path.display()
        )));
    }
    let bytes = fs::read(&canonical).map_err(|error| {
        BackendError::environment(format!(
            "failed to read locked environment input {}: {error}",
            path.display()
        ))
    })?;
    Ok(sha256_fingerprint(bytes))
}

pub(crate) fn host_architecture() -> Result<Architecture, BackendError> {
    architecture_from_name(env::consts::ARCH)
}

fn architecture_from_name(value: &str) -> Result<Architecture, BackendError> {
    match value {
        "x86_64" => Ok(Architecture::Amd64),
        "aarch64" => Ok(Architecture::Arm64),
        unsupported => Err(BackendError::environment(format!(
            "unsupported host architecture {unsupported}; managed environments support x86_64 and aarch64"
        ))),
    }
}

pub(crate) const fn platform_architecture(architecture: Architecture) -> &'static str {
    match architecture {
        Architecture::Amd64 => "amd64",
        Architecture::Arm64 => "arm64",
    }
}

fn validate_reference(reference: &str) -> Result<(), BackendError> {
    if reference.trim().is_empty()
        || reference.contains(char::is_whitespace)
        || reference.contains('@')
        || reference.starts_with('-')
    {
        Err(BackendError::input("invalid OCI base reference"))
    } else {
        Ok(())
    }
}

fn validate_digest(digest: &str) -> Result<(), BackendError> {
    let valid = digest
        .strip_prefix("sha256:")
        .is_some_and(|hex| hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()));
    if valid {
        Ok(())
    } else {
        Err(BackendError::input(
            "OCI base digest must be sha256 followed by 64 hexadecimal characters",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_tool_baseline_changes_invalidate_locks_without_comparing_source_paths() {
        use ayni_core::*;
        let source = RequirementSource::new(
            "adapter",
            "Cargo.toml",
            None::<String>,
            RequirementConfidence::Declared,
        )
        .unwrap();
        let mut planned = SignalToolRequirement {
            tool: "analysis".into(),
            version: VersionRequirement::exact("1.2.3").unwrap(),
            version_authority: ToolVersionAuthority::AdapterPinned,
            provider: "cargo-install".into(),
            scope: ToolInstallationScope::Isolated,
            signals: vec![SignalKind::Complexity],
            supported_platforms: vec![],
            provisioning: ProvisioningSupport::OnlineOnly,
            modifies_checkout: false,
            source,
        };
        let mut locked = LockedSignalTool {
            tool: planned.tool.clone(),
            version: "1.2.3".into(),
            version_authority: planned.version_authority,
            provider: planned.provider.clone(),
            scope: planned.scope,
            signals: planned.signals.clone(),
            source: LockedRequirementSource {
                kind: "native_lock".into(),
                path: "Cargo.lock".into(),
                digest: None,
                confidence: RequirementConfidence::Exact,
            },
        };
        assert!(signal_tool_matches_lock(&planned, &locked));
        locked.version = "1.2.4".into();
        assert!(!signal_tool_matches_lock(&planned, &locked));
        planned.version = VersionRequirement::selector("1.2").unwrap();
        planned.version_authority = ToolVersionAuthority::LockResolved;
        assert!(!signal_tool_matches_lock(&planned, &locked));
        locked.version_authority = ToolVersionAuthority::LockResolved;
        assert!(signal_tool_matches_lock(&planned, &locked));
    }

    #[test]
    fn host_architecture_mapping_rejects_unsupported_targets() {
        assert_eq!(
            architecture_from_name("x86_64").expect("amd64"),
            Architecture::Amd64
        );
        assert_eq!(
            architecture_from_name("aarch64").expect("arm64"),
            Architecture::Arm64
        );
        let error = architecture_from_name("riscv64").expect_err("unsupported architecture");
        assert_eq!(error.kind, crate::BackendErrorKind::Environment);
        assert!(
            error
                .message
                .contains("unsupported host architecture riscv64")
        );
    }
}
