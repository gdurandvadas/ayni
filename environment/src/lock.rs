use crate::{BackendError, concise_output};
use ayni_adapters_common::exec::run_command;
use ayni_core::{Architecture, EnvironmentLock, EnvironmentPlan, ProvisioningBase};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::path::Path;
use std::time::Duration;

pub const LOCK_FILE: &str = ".ayni.lock";
pub const BASE_VARIANT: &str = "debian";
pub const BASE_MISE_VERSION: &str = "2025.2.4";
const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);

/// Resolve the published base to an immutable manifest digest. An explicit
/// value must use `<reference>@sha256:<digest>`.
pub fn resolve_provisioning_base(
    ayni_version: &str,
    explicit: Option<&str>,
) -> Result<ProvisioningBase, BackendError> {
    let (reference, digest) = if let Some(explicit) = explicit {
        parse_exact_base(explicit)?
    } else {
        let reference = format!("ghcr.io/gdurandvadas/ayni-env:{ayni_version}-{BASE_VARIANT}");
        let digest = inspect_remote_digest(&reference)?;
        (reference, digest)
    };
    Ok(ProvisioningBase {
        reference,
        digest,
        variant: BASE_VARIANT.to_owned(),
        mise_version: BASE_MISE_VERSION.to_owned(),
    })
}

fn parse_exact_base(value: &str) -> Result<(String, String), BackendError> {
    let (reference, digest) = value
        .rsplit_once('@')
        .ok_or_else(|| BackendError::input("--base must use <reference>@sha256:<digest>"))?;
    validate_reference(reference)?;
    validate_digest(digest)?;
    Ok((reference.to_owned(), digest.to_ascii_lowercase()))
}

fn inspect_remote_digest(reference: &str) -> Result<String, BackendError> {
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
            "failed to resolve immutable environment base {reference}: {error}; install Docker Buildx or pass `--base <reference>@sha256:<digest>`"
        ))
    })?;
    if !output.status.success() {
        return Err(BackendError::environment(format!(
            "failed to resolve immutable environment base {reference}: {}; pass an available exact image with `--base`",
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
                "Docker returned no manifest digest for environment base {reference}"
            ))
        })?;
    validate_digest(digest)?;
    Ok(digest.to_ascii_lowercase())
}

/// Compare current adapter discovery with a validated lock without requiring
/// unresolved selectors to equal their locked exact versions.
pub fn plan_matches_lock(plan: &EnvironmentPlan, lock: &EnvironmentLock) -> bool {
    plan.repository().contract_digest == lock.repository().contract_digest
        && plan.targets().len() == lock.targets().len()
        && plan
            .targets()
            .iter()
            .zip(lock.targets())
            .all(|(plan, locked)| {
                plan.target == locked.target
                    && plan.runtimes.len() == locked.runtimes.len()
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
                        .all(|(left, right)| {
                            left.tool == right.tool
                                && left.provider == right.provider
                                && left.scope == right.scope
                                && left.signals == right.signals
                                && left.source.path == right.source.path
                        })
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

pub fn read_lock(repo_root: &Path) -> Result<EnvironmentLock, BackendError> {
    let path = repo_root.join(LOCK_FILE);
    let metadata = fs::symlink_metadata(&path).map_err(|error| {
        BackendError::environment(format!(
            "environment lock {} is required: {error}; run `ayni env lock`",
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
            "environment lock {} is invalid: {error}; run `ayni env lock`",
            path.display()
        ))
    })?;
    validate_lock(repo_root, &lock)?;
    Ok(lock)
}

fn validate_lock(repo_root: &Path, lock: &EnvironmentLock) -> Result<(), BackendError> {
    if lock.ayni_version() != env!("CARGO_PKG_VERSION") {
        return Err(BackendError::environment(format!(
            "environment lock was created by Ayni {}, but this binary is {}; run `ayni env lock`",
            lock.ayni_version(),
            env!("CARGO_PKG_VERSION")
        )));
    }
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
            "failed to inspect locked environment input {}: {error}; run `ayni env lock`",
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
    Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
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
