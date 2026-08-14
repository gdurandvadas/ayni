use crate::{
    EnvironmentPlanError, RequirementConfidence, RequirementSource, ResolvedEnvironmentPlan,
    SignalKind, TargetIdentity, TargetPlatform, ToolInstallationScope, VersionRequirement,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Component, Path};

/// Version of the committed, deterministic environment lock document.
pub const ENVIRONMENT_LOCK_SCHEMA_VERSION: &str = "0.2.0";

/// Immutable OCI base selected by the environment backend. The reference is
/// human-readable while the digest is the authoritative image identity.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProvisioningBase {
    pub reference: String,
    pub digest: String,
    pub variant: String,
    pub mise_version: String,
}

/// Portable provenance retained by a lock. Free-form source detail is omitted
/// because it can contain host-specific diagnostics or executable text.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LockedRequirementSource {
    pub kind: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub digest: Option<String>,
    pub confidence: RequirementConfidence,
}

impl LockedRequirementSource {
    fn from_source(source: &RequirementSource, digests: &BTreeMap<String, String>) -> Self {
        Self {
            kind: source.kind.clone(),
            path: source.path.clone(),
            digest: digests.get(&source.path).cloned(),
            confidence: source.confidence,
        }
    }
}

/// Exact lock projection for one runtime requirement.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LockedRuntime {
    pub runtime: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub components: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub targets: Vec<String>,
    pub source: LockedRequirementSource,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LockedPackageManager {
    pub family: String,
    pub version: String,
    pub ownership_root: String,
    pub source: LockedRequirementSource,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LockedSignalTool {
    pub tool: String,
    pub version: String,
    pub provider: String,
    pub scope: ToolInstallationScope,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub signals: Vec<SignalKind>,
    pub source: LockedRequirementSource,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LockedDependencyLock {
    pub path: String,
    pub digest: String,
    pub owner_root: String,
    pub source: LockedRequirementSource,
}

/// Exact, portable projection of a resolved target. This intentionally omits
/// system commands, host paths, credentials, and checkout-mutating details.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LockedTargetEnvironment {
    pub target: TargetIdentity,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub runtimes: Vec<LockedRuntime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package_manager: Option<LockedPackageManager>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub signal_tools: Vec<LockedSignalTool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dependency_locks: Vec<LockedDependencyLock>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LockedRepositoryIdentity {
    pub contract_path: String,
    pub contract_digest: String,
}

/// Versioned, self-authenticating, deterministic lock projection. The
/// fingerprint is SHA-256 over the canonical JSON representation with this
/// field omitted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentLock {
    schema_version: String,
    repository: LockedRepositoryIdentity,
    ayni_version: String,
    mise_version: String,
    provisioning_base: ProvisioningBase,
    platforms: Vec<TargetPlatform>,
    targets: Vec<LockedTargetEnvironment>,
    fingerprint: String,
}

impl<'de> Deserialize<'de> for EnvironmentLock {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            schema_version: String,
            repository: LockedRepositoryIdentity,
            ayni_version: String,
            mise_version: String,
            provisioning_base: ProvisioningBase,
            platforms: Vec<TargetPlatform>,
            targets: Vec<LockedTargetEnvironment>,
            fingerprint: String,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::from_parts(EnvironmentLockParts {
            repository: wire.repository,
            ayni_version: wire.ayni_version,
            mise_version: wire.mise_version,
            provisioning_base: wire.provisioning_base,
            platforms: wire.platforms,
            targets: wire.targets,
            fingerprint: wire.fingerprint,
            schema_version: Some(wire.schema_version),
        })
        .map_err(serde::de::Error::custom)
    }
}

struct EnvironmentLockParts {
    repository: LockedRepositoryIdentity,
    ayni_version: String,
    mise_version: String,
    provisioning_base: ProvisioningBase,
    platforms: Vec<TargetPlatform>,
    targets: Vec<LockedTargetEnvironment>,
    fingerprint: String,
    schema_version: Option<String>,
}

impl EnvironmentLock {
    /// Projects a fully resolved plan and immutable provisioning base into a lock.
    pub fn from_resolved_plan(
        plan: &ResolvedEnvironmentPlan,
        ayni_version: impl Into<String>,
        mise_version: impl Into<String>,
        provisioning_base: ProvisioningBase,
        contract_path: impl AsRef<str>,
        source_digests: &BTreeMap<String, String>,
    ) -> Result<Self, EnvironmentPlanError> {
        let plan = plan.plan();
        if let Some(target) = plan
            .targets()
            .iter()
            .find(|target| !target.system_requirements.is_empty())
        {
            return Err(EnvironmentPlanError::UnsupportedProvisioning {
                target: target.target.clone(),
                item: String::from("locked system requirements"),
            });
        }
        let targets = plan
            .targets()
            .iter()
            .map(|target| LockedTargetEnvironment {
                target: target.target.clone(),
                runtimes: target
                    .runtimes
                    .iter()
                    .map(|runtime| LockedRuntime {
                        runtime: runtime.runtime.clone(),
                        version: exact_version(&runtime.version)
                            .expect("resolved plans have exact versions"),
                        components: runtime.components.clone(),
                        targets: runtime.targets.clone(),
                        source: LockedRequirementSource::from_source(
                            &runtime.source,
                            source_digests,
                        ),
                    })
                    .collect(),
                package_manager: target.package_manager.as_ref().map(|manager| {
                    LockedPackageManager {
                        family: manager.family.clone(),
                        version: exact_version(&manager.version)
                            .expect("resolved plans have exact versions"),
                        ownership_root: manager.ownership_root.clone(),
                        source: LockedRequirementSource::from_source(
                            &manager.source,
                            source_digests,
                        ),
                    }
                }),
                signal_tools: target
                    .signal_tools
                    .iter()
                    .map(|tool| LockedSignalTool {
                        tool: tool.tool.clone(),
                        version: exact_version(&tool.version)
                            .expect("resolved plans have exact versions"),
                        provider: tool.provider.clone(),
                        scope: tool.scope,
                        signals: tool.signals.clone(),
                        source: LockedRequirementSource::from_source(&tool.source, source_digests),
                    })
                    .collect(),
                dependency_locks: target
                    .dependency_locks
                    .iter()
                    .map(|lock| LockedDependencyLock {
                        path: lock.path.clone(),
                        digest: lock.digest.clone(),
                        owner_root: lock.owner_root.clone(),
                        source: LockedRequirementSource::from_source(&lock.source, source_digests),
                    })
                    .collect(),
            })
            .collect();
        Self::from_parts(EnvironmentLockParts {
            repository: LockedRepositoryIdentity {
                contract_path: contract_path.as_ref().to_owned(),
                contract_digest: plan.repository().contract_digest.clone(),
            },
            ayni_version: ayni_version.into(),
            mise_version: mise_version.into(),
            provisioning_base,
            platforms: plan.platforms().to_vec(),
            targets,
            fingerprint: String::new(),
            schema_version: None,
        })
    }

    fn from_parts(parts: EnvironmentLockParts) -> Result<Self, EnvironmentPlanError> {
        let EnvironmentLockParts {
            mut repository,
            mut ayni_version,
            mut mise_version,
            mut provisioning_base,
            mut platforms,
            mut targets,
            fingerprint,
            schema_version,
        } = parts;
        let deserializing = schema_version.is_some();
        if let Some(schema_version) = schema_version
            && schema_version != ENVIRONMENT_LOCK_SCHEMA_VERSION
        {
            return Err(EnvironmentPlanError::UnsupportedLockSchema(schema_version));
        }
        normalize_provisioning_base(&mut provisioning_base)?;
        normalize_lock_header(
            &mut repository,
            &mut ayni_version,
            &mut mise_version,
            &mut platforms,
        )?;
        normalize_locked_targets(&mut targets)?;
        let mut lock = Self {
            schema_version: ENVIRONMENT_LOCK_SCHEMA_VERSION.to_owned(),
            repository,
            ayni_version,
            mise_version,
            provisioning_base,
            platforms,
            targets,
            fingerprint: String::new(),
        };
        let expected = lock.computed_fingerprint()?;
        if (deserializing && fingerprint.is_empty())
            || (!fingerprint.is_empty() && fingerprint != expected)
        {
            return Err(EnvironmentPlanError::FingerprintMismatch);
        }
        lock.fingerprint = expected;
        Ok(lock)
    }

    /// Canonical compact JSON with exactly one trailing newline.
    pub fn canonical_json(&self) -> Result<String, EnvironmentPlanError> {
        let mut normalized = self.clone();
        normalized.fingerprint = normalized.computed_fingerprint()?;
        serde_json::to_string(&normalized)
            .map(|json| format!("{json}\n"))
            .map_err(|error| EnvironmentPlanError::Serialization(error.to_string()))
    }

    fn computed_fingerprint(&self) -> Result<String, EnvironmentPlanError> {
        #[derive(Serialize)]
        struct FingerprintDocument<'a> {
            schema_version: &'a str,
            repository: &'a LockedRepositoryIdentity,
            ayni_version: &'a str,
            mise_version: &'a str,
            provisioning_base: &'a ProvisioningBase,
            platforms: &'a [TargetPlatform],
            targets: &'a [LockedTargetEnvironment],
        }
        let document = FingerprintDocument {
            schema_version: &self.schema_version,
            repository: &self.repository,
            ayni_version: &self.ayni_version,
            mise_version: &self.mise_version,
            provisioning_base: &self.provisioning_base,
            platforms: &self.platforms,
            targets: &self.targets,
        };
        let bytes = serde_json::to_vec(&document)
            .map_err(|error| EnvironmentPlanError::Serialization(error.to_string()))?;
        Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
    }

    #[must_use]
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }
    #[must_use]
    pub fn repository(&self) -> &LockedRepositoryIdentity {
        &self.repository
    }
    #[must_use]
    pub fn ayni_version(&self) -> &str {
        &self.ayni_version
    }
    #[must_use]
    pub fn mise_version(&self) -> &str {
        &self.mise_version
    }
    #[must_use]
    pub fn platforms(&self) -> &[TargetPlatform] {
        &self.platforms
    }
    #[must_use]
    pub fn targets(&self) -> &[LockedTargetEnvironment] {
        &self.targets
    }
    #[must_use]
    pub fn provisioning_base(&self) -> &ProvisioningBase {
        &self.provisioning_base
    }
}

fn normalize_provisioning_base(base: &mut ProvisioningBase) -> Result<(), EnvironmentPlanError> {
    base.reference = lock_required_label("provisioning-base reference", base.reference.clone())?;
    if base.reference.contains(char::is_whitespace) || base.reference.contains('@') {
        return Err(EnvironmentPlanError::EmptyField(
            "provisioning-base reference",
        ));
    }
    base.digest = lock_validate_digest("provisioning-base digest", base.digest.clone())?;
    base.variant = lock_required_label("provisioning-base variant", base.variant.clone())?;
    base.mise_version =
        normalize_exact_lock_version("provisioning-base mise version", base.mise_version.clone())?;
    Ok(())
}

fn normalize_lock_header(
    repository: &mut LockedRepositoryIdentity,
    ayni_version: &mut String,
    mise_version: &mut String,
    platforms: &mut Vec<TargetPlatform>,
) -> Result<(), EnvironmentPlanError> {
    repository.contract_path =
        lock_normalize_repository_path("contract path", &repository.contract_path)?;
    repository.contract_digest =
        lock_validate_digest("contract digest", repository.contract_digest.clone())?;
    *ayni_version = normalize_exact_lock_version("ayni version", ayni_version.clone())?;
    *mise_version = normalize_exact_lock_version("mise version", mise_version.clone())?;
    if platforms.is_empty() {
        return Err(EnvironmentPlanError::MissingPlatforms);
    }
    platforms.sort();
    platforms.dedup();
    Ok(())
}

fn normalize_locked_targets(
    targets: &mut [LockedTargetEnvironment],
) -> Result<(), EnvironmentPlanError> {
    if targets.is_empty() {
        return Err(EnvironmentPlanError::MissingTargets);
    }
    for target in targets.iter_mut() {
        normalize_locked_target(target)?;
    }
    targets.sort_by(|left, right| left.target.cmp(&right.target));
    if let Some(duplicate) = targets
        .windows(2)
        .find(|pair| pair[0].target == pair[1].target)
    {
        return Err(EnvironmentPlanError::DuplicateTarget(
            duplicate[0].target.clone(),
        ));
    }
    Ok(())
}

fn exact_version(requirement: &VersionRequirement) -> Option<String> {
    match requirement {
        VersionRequirement::Exact { version } => Some(version.clone()),
        _ => None,
    }
}

fn normalize_exact_lock_version(
    field: &'static str,
    value: String,
) -> Result<String, EnvironmentPlanError> {
    let value = lock_required_label(field, value)?;
    lock_reject_floating_version(&value)?;
    Ok(value)
}

fn normalize_locked_source(
    source: &mut LockedRequirementSource,
) -> Result<(), EnvironmentPlanError> {
    source.kind = lock_required_label("source kind", source.kind.clone())?;
    source.path = lock_normalize_repository_path("requirement source", &source.path)?;
    if let Some(digest) = source.digest.take() {
        source.digest = Some(lock_validate_digest("requirement-source digest", digest)?);
    } else if source.path != "." {
        return Err(EnvironmentPlanError::MissingSourceDigest(
            source.path.clone(),
        ));
    }
    Ok(())
}

fn normalize_locked_target(
    target: &mut LockedTargetEnvironment,
) -> Result<(), EnvironmentPlanError> {
    target.target.root = lock_normalize_repository_path("target root", &target.target.root)?;
    if target.runtimes.is_empty() {
        return Err(EnvironmentPlanError::MissingRuntime(target.target.clone()));
    }
    normalize_locked_runtimes(&mut target.runtimes)?;
    if let Some(manager) = &mut target.package_manager {
        normalize_locked_manager(manager, &target.target)?;
    }
    normalize_locked_tools(&mut target.signal_tools)?;
    normalize_locked_dependency_locks(&mut target.dependency_locks, &target.target)?;
    Ok(())
}

fn normalize_locked_runtimes(
    runtimes: &mut Vec<LockedRuntime>,
) -> Result<(), EnvironmentPlanError> {
    for runtime in runtimes.iter_mut() {
        runtime.runtime = lock_required_label("runtime", runtime.runtime.clone())?;
        runtime.version = normalize_exact_lock_version("runtime version", runtime.version.clone())?;
        lock_normalize_string_set("runtime component", &mut runtime.components)?;
        lock_normalize_string_set("runtime target", &mut runtime.targets)?;
        normalize_locked_source(&mut runtime.source)?;
    }
    runtimes.sort();
    runtimes.dedup();
    Ok(())
}

fn normalize_locked_manager(
    manager: &mut LockedPackageManager,
    target: &TargetIdentity,
) -> Result<(), EnvironmentPlanError> {
    manager.family = lock_required_label("package manager", manager.family.clone())?;
    manager.version =
        normalize_exact_lock_version("package-manager version", manager.version.clone())?;
    manager.ownership_root =
        lock_normalize_repository_path("package-manager ownership root", &manager.ownership_root)?;
    normalize_locked_source(&mut manager.source)?;
    lock_ensure_contains(
        "package-manager ownership root",
        &manager.ownership_root,
        &target.root,
    )
}

fn normalize_locked_tools(tools: &mut Vec<LockedSignalTool>) -> Result<(), EnvironmentPlanError> {
    for tool in tools.iter_mut() {
        tool.tool = lock_required_label("signal tool", tool.tool.clone())?;
        tool.version = normalize_exact_lock_version("signal-tool version", tool.version.clone())?;
        tool.provider = lock_required_label("signal-tool provider", tool.provider.clone())?;
        tool.signals.sort();
        tool.signals.dedup();
        normalize_locked_source(&mut tool.source)?;
    }
    tools.sort();
    tools.dedup();
    Ok(())
}

fn normalize_locked_dependency_locks(
    locks: &mut Vec<LockedDependencyLock>,
    target: &TargetIdentity,
) -> Result<(), EnvironmentPlanError> {
    for lock in locks.iter_mut() {
        lock.path = lock_normalize_repository_path("dependency lock", &lock.path)?;
        lock.digest = lock_validate_digest("dependency-lock digest", lock.digest.clone())?;
        lock.owner_root =
            lock_normalize_repository_path("dependency-lock owner", &lock.owner_root)?;
        normalize_locked_source(&mut lock.source)?;
        lock_ensure_contains("dependency lock", &lock.owner_root, &lock.path)?;
        lock_ensure_contains("dependency-lock owner", &lock.owner_root, &target.root)?;
    }
    locks.sort();
    locks.dedup();
    Ok(())
}

fn lock_required_label(field: &'static str, value: String) -> Result<String, EnvironmentPlanError> {
    let value = value.trim().to_owned();
    if value.is_empty() {
        Err(EnvironmentPlanError::EmptyField(field))
    } else {
        Ok(value)
    }
}

fn lock_reject_floating_version(value: &str) -> Result<(), EnvironmentPlanError> {
    crate::environment::reject_floating_version(value)
}

fn lock_validate_digest(
    field: &'static str,
    value: String,
) -> Result<String, EnvironmentPlanError> {
    let value = value.trim().to_ascii_lowercase();
    let valid = value
        .strip_prefix("sha256:")
        .is_some_and(|hex| hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()));
    if valid {
        Ok(value)
    } else {
        Err(EnvironmentPlanError::InvalidDigest { field, value })
    }
}

fn lock_normalize_string_set(
    field: &'static str,
    values: &mut Vec<String>,
) -> Result<(), EnvironmentPlanError> {
    for value in values.iter_mut() {
        *value = lock_required_label(field, value.clone())?;
    }
    values.sort();
    values.dedup();
    Ok(())
}

fn lock_normalize_repository_path(
    field: &'static str,
    value: &str,
) -> Result<String, EnvironmentPlanError> {
    let normalized = value.replace('\\', "/");
    if normalized == "." {
        return Ok(normalized);
    }
    let path = Path::new(&normalized);
    let drive = normalized.as_bytes().get(1) == Some(&b':');
    if path.is_absolute()
        || drive
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(EnvironmentPlanError::NonPortablePath {
            field,
            value: value.to_owned(),
        });
    }
    lock_required_label(field, normalized)
}

fn lock_ensure_contains(
    field: &'static str,
    owner: &str,
    path: &str,
) -> Result<(), EnvironmentPlanError> {
    let owner_path = Path::new(owner);
    let path = Path::new(path);
    if owner == "." || path == owner_path || path.starts_with(owner_path) {
        Ok(())
    } else {
        Err(EnvironmentPlanError::PathOutsideOwner {
            field,
            owner: owner.to_owned(),
            path: path.to_string_lossy().into_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Architecture, Language, Libc, OperatingSystem};

    fn digest(character: char) -> String {
        format!("sha256:{}", character.to_string().repeat(64))
    }

    fn base() -> ProvisioningBase {
        ProvisioningBase {
            reference: "ghcr.io/gdurandvadas/ayni-env:0.8.1-debian".to_owned(),
            digest: digest('b'),
            variant: "debian".to_owned(),
            mise_version: "2025.2.4".to_owned(),
        }
    }

    fn target(language: Language, root: &str) -> LockedTargetEnvironment {
        LockedTargetEnvironment {
            target: TargetIdentity::new(language, root).expect("target identity"),
            runtimes: vec![LockedRuntime {
                runtime: language.to_string(),
                version: "1.2.3".to_owned(),
                components: Vec::new(),
                targets: Vec::new(),
                source: LockedRequirementSource {
                    kind: "test".to_owned(),
                    path: ".".to_owned(),
                    digest: None,
                    confidence: RequirementConfidence::Exact,
                },
            }],
            package_manager: None,
            signal_tools: Vec::new(),
            dependency_locks: Vec::new(),
        }
    }

    fn lock(targets: Vec<LockedTargetEnvironment>) -> EnvironmentLock {
        EnvironmentLock::from_parts(EnvironmentLockParts {
            repository: LockedRepositoryIdentity {
                contract_path: ".ayni.toml".to_owned(),
                contract_digest: digest('a'),
            },
            ayni_version: "0.8.1".to_owned(),
            mise_version: "2026.8.7".to_owned(),
            provisioning_base: base(),
            platforms: vec![TargetPlatform {
                os: OperatingSystem::Linux,
                architecture: Architecture::Amd64,
                libc: Libc::Glibc,
            }],
            targets,
            fingerprint: String::new(),
            schema_version: None,
        })
        .expect("valid lock")
    }

    #[test]
    fn canonical_serialization_is_sorted_and_round_trips() {
        let lock = lock(vec![
            target(Language::Rust, "z"),
            target(Language::Node, "a"),
        ]);
        let serialized = lock.canonical_json().expect("canonical JSON");
        assert!(serialized.ends_with('\n'));
        assert!(!serialized.ends_with("\n\n"));
        let parsed: EnvironmentLock = serde_json::from_str(&serialized).expect("valid lock JSON");
        assert_eq!(parsed.canonical_json().expect("canonical JSON"), serialized);
        assert_eq!(parsed.targets()[0].target.language, Language::Rust);
    }

    #[test]
    fn construction_rejects_non_immutable_provisioning_base() {
        let mut base = base();
        base.digest = "latest".to_owned();
        assert!(
            EnvironmentLock::from_parts(EnvironmentLockParts {
                repository: LockedRepositoryIdentity {
                    contract_path: ".ayni.toml".to_owned(),
                    contract_digest: digest('a')
                },
                ayni_version: "0.8.1".to_owned(),
                mise_version: "2026.8.7".to_owned(),
                provisioning_base: base,
                platforms: vec![TargetPlatform {
                    os: OperatingSystem::Linux,
                    architecture: Architecture::Amd64,
                    libc: Libc::Glibc
                }],
                targets: vec![target(Language::Rust, ".")],
                fingerprint: String::new(),
                schema_version: None,
            })
            .is_err()
        );
    }

    #[test]
    fn deserialization_rejects_fingerprint_tampering() {
        let serialized = lock(vec![target(Language::Rust, ".")])
            .canonical_json()
            .expect("canonical JSON");
        let tampered =
            serialized.replace("\"ayni_version\":\"0.8.1\"", "\"ayni_version\":\"0.8.2\"");
        assert_ne!(tampered, serialized);
        assert!(serde_json::from_str::<EnvironmentLock>(&tampered).is_err());
    }

    #[test]
    fn construction_rejects_selector_syntax_in_exact_lock_versions() {
        let mut target = target(Language::Rust, ".");
        target.runtimes[0].version = "^1.2.3".to_owned();
        assert!(
            EnvironmentLock::from_parts(EnvironmentLockParts {
                repository: LockedRepositoryIdentity {
                    contract_path: ".ayni.toml".to_owned(),
                    contract_digest: digest('a'),
                },
                ayni_version: "0.8.1".to_owned(),
                mise_version: "2026.8.7".to_owned(),
                provisioning_base: base(),
                platforms: vec![TargetPlatform {
                    os: OperatingSystem::Linux,
                    architecture: Architecture::Amd64,
                    libc: Libc::Glibc,
                }],
                targets: vec![target],
                fingerprint: String::new(),
                schema_version: None,
            })
            .is_err()
        );
    }

    #[test]
    fn deserialization_rejects_an_empty_fingerprint() {
        let serialized = lock(vec![target(Language::Rust, ".")])
            .canonical_json()
            .expect("canonical JSON");
        let fingerprint = lock(vec![target(Language::Rust, ".")])
            .fingerprint()
            .to_owned();
        let empty = serialized.replace(&fingerprint, "");
        assert_ne!(empty, serialized);
        assert!(serde_json::from_str::<EnvironmentLock>(&empty).is_err());
    }

    #[test]
    fn lock_repository_identity_excludes_checkout_name() {
        let lock = lock(vec![target(Language::Rust, ".")]);
        let serialized = lock.canonical_json().expect("canonical JSON");
        assert!(!serialized.contains("repository name"));
        assert_eq!(lock.repository().contract_digest, digest('a'));
    }
}
