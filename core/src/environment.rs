//! Portable semantic contracts for repository environment planning.
//!
//! Adapters own ecosystem interpretation and produce these values. Provisioning
//! backends consume validated plans without re-reading language manifests or
//! embedding provider-specific commands in core.

use crate::{Language, SignalKind};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::{Component, Path, PathBuf};

/// Version of the clean-slate, explainable environment-plan document.
pub const ENVIRONMENT_PLAN_SCHEMA_VERSION: &str = "0.1.0";

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct RepositoryIdentity {
    pub name: String,
    pub contract_digest: String,
}

/// Stable identity for one configured adapter target. Host checkout paths are
/// deliberately excluded.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct TargetIdentity {
    pub language: Language,
    pub root: String,
}

impl TargetIdentity {
    pub fn new(language: Language, root: impl AsRef<str>) -> Result<Self, EnvironmentPlanError> {
        Ok(Self {
            language,
            root: normalize_repository_path("target root", root.as_ref())?,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequirementConfidence {
    Assumed,
    Inferred,
    Declared,
    Exact,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct RequirementSource {
    pub kind: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub confidence: RequirementConfidence,
}

impl RequirementSource {
    pub fn new(
        kind: impl Into<String>,
        path: impl AsRef<str>,
        detail: Option<impl Into<String>>,
        confidence: RequirementConfidence,
    ) -> Result<Self, EnvironmentPlanError> {
        Ok(Self {
            kind: required_label("source kind", kind.into())?,
            path: normalize_repository_path("requirement source", path.as_ref())?,
            detail: normalize_optional_label("source detail", detail.map(Into::into))?,
            confidence,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum VersionRequirement {
    Exact { version: String },
    Selector { expression: String },
    Compatibility { expression: String },
    Minimum { version: String },
    Unresolved { reason: String },
}

impl VersionRequirement {
    pub fn exact(version: impl Into<String>) -> Result<Self, EnvironmentPlanError> {
        let version = required_label("exact version", version.into())?;
        reject_floating_version(&version)?;
        Ok(Self::Exact { version })
    }

    pub fn selector(expression: impl Into<String>) -> Result<Self, EnvironmentPlanError> {
        Ok(Self::Selector {
            expression: required_label("version selector", expression.into())?,
        })
    }

    pub fn compatibility(expression: impl Into<String>) -> Result<Self, EnvironmentPlanError> {
        Ok(Self::Compatibility {
            expression: required_label("version compatibility", expression.into())?,
        })
    }

    pub fn minimum(version: impl Into<String>) -> Result<Self, EnvironmentPlanError> {
        Ok(Self::Minimum {
            version: required_label("minimum version", version.into())?,
        })
    }

    pub fn unresolved(reason: impl Into<String>) -> Result<Self, EnvironmentPlanError> {
        Ok(Self::Unresolved {
            reason: required_label("unresolved reason", reason.into())?,
        })
    }

    #[must_use]
    pub const fn is_exact(&self) -> bool {
        matches!(self, Self::Exact { .. })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct RuntimeRequirement {
    pub runtime: String,
    pub version: VersionRequirement,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub components: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub targets: Vec<String>,
    pub source: RequirementSource,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PackageManagerRequirement {
    pub family: String,
    pub version: VersionRequirement,
    pub ownership_root: String,
    pub source: RequirementSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolInstallationScope {
    Runtime,
    Isolated,
    Project,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProvisioningSupport {
    LockedOffline,
    OnlineOnly,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SignalToolRequirement {
    pub tool: String,
    pub version: VersionRequirement,
    pub provider: String,
    pub scope: ToolInstallationScope,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub signals: Vec<SignalKind>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supported_platforms: Vec<TargetPlatform>,
    pub provisioning: ProvisioningSupport,
    pub modifies_checkout: bool,
    pub source: RequirementSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SystemRequirementKind {
    Capability,
    Package,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SystemRequirement {
    pub kind: SystemRequirementKind,
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supported_platforms: Vec<TargetPlatform>,
    pub provisioning: ProvisioningSupport,
    pub source: RequirementSource,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DependencyLockRequirement {
    pub path: String,
    pub digest: String,
    pub owner_root: String,
    pub source: RequirementSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperatingSystem {
    Linux,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Architecture {
    Amd64,
    Arm64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Libc {
    Glibc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct TargetPlatform {
    pub os: OperatingSystem,
    pub architecture: Architecture,
    pub libc: Libc,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetEnvironment {
    pub target: TargetIdentity,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub runtimes: Vec<RuntimeRequirement>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package_manager: Option<PackageManagerRequirement>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub signal_tools: Vec<SignalToolRequirement>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub system_requirements: Vec<SystemRequirement>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dependency_locks: Vec<DependencyLockRequirement>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct EnvironmentWarning {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<TargetIdentity>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct EnvironmentConflict {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<TargetIdentity>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<RequirementSource>,
}

/// Explainable plan. This value may intentionally contain unresolved
/// requirements and blocking conflicts so `env show` can report them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EnvironmentPlan {
    schema_version: String,
    repository: RepositoryIdentity,
    platforms: Vec<TargetPlatform>,
    targets: Vec<TargetEnvironment>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    warnings: Vec<EnvironmentWarning>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    conflicts: Vec<EnvironmentConflict>,
}

impl<'de> Deserialize<'de> for EnvironmentPlan {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Wire {
            schema_version: String,
            repository: RepositoryIdentity,
            platforms: Vec<TargetPlatform>,
            targets: Vec<TargetEnvironment>,
            #[serde(default)]
            warnings: Vec<EnvironmentWarning>,
            #[serde(default)]
            conflicts: Vec<EnvironmentConflict>,
        }

        let wire = Wire::deserialize(deserializer)?;
        if wire.schema_version != ENVIRONMENT_PLAN_SCHEMA_VERSION {
            return Err(serde::de::Error::custom(format!(
                "unsupported environment plan schema {}; expected {ENVIRONMENT_PLAN_SCHEMA_VERSION}",
                wire.schema_version
            )));
        }
        Self::new(
            wire.repository,
            wire.platforms,
            wire.targets,
            wire.warnings,
            wire.conflicts,
        )
        .map_err(serde::de::Error::custom)
    }
}

impl EnvironmentPlan {
    pub fn new(
        repository: RepositoryIdentity,
        platforms: Vec<TargetPlatform>,
        targets: Vec<TargetEnvironment>,
        warnings: Vec<EnvironmentWarning>,
        conflicts: Vec<EnvironmentConflict>,
    ) -> Result<Self, EnvironmentPlanError> {
        let mut plan = Self {
            schema_version: String::from(ENVIRONMENT_PLAN_SCHEMA_VERSION),
            repository,
            platforms,
            targets,
            warnings,
            conflicts,
        };
        plan.normalize_and_validate()?;
        Ok(plan)
    }

    #[must_use]
    pub fn repository(&self) -> &RepositoryIdentity {
        &self.repository
    }

    #[must_use]
    pub fn platforms(&self) -> &[TargetPlatform] {
        &self.platforms
    }

    #[must_use]
    pub fn targets(&self) -> &[TargetEnvironment] {
        &self.targets
    }

    #[must_use]
    pub fn warnings(&self) -> &[EnvironmentWarning] {
        &self.warnings
    }

    #[must_use]
    pub fn conflicts(&self) -> &[EnvironmentConflict] {
        &self.conflicts
    }

    pub fn resolve(self) -> Result<ResolvedEnvironmentPlan, EnvironmentPlanError> {
        if !self.conflicts.is_empty() {
            return Err(EnvironmentPlanError::BlockingConflicts(
                self.conflicts.len(),
            ));
        }
        let unresolved = self.unresolved_requirements();
        if unresolved != 0 {
            return Err(EnvironmentPlanError::UnresolvedRequirements(unresolved));
        }
        self.validate_provisioning_readiness()?;
        Ok(ResolvedEnvironmentPlan(self))
    }

    fn normalize_and_validate(&mut self) -> Result<(), EnvironmentPlanError> {
        self.repository.name = required_label("repository name", self.repository.name.clone())?;
        self.repository.contract_digest =
            validate_digest("contract digest", self.repository.contract_digest.clone())?;
        if self.platforms.is_empty() {
            return Err(EnvironmentPlanError::MissingPlatforms);
        }
        self.platforms.sort();
        self.platforms.dedup();
        if self.targets.is_empty() {
            return Err(EnvironmentPlanError::MissingTargets);
        }

        for target in &mut self.targets {
            normalize_target_environment(target)?;
        }
        self.targets
            .sort_by(|left, right| left.target.cmp(&right.target));
        for pair in self.targets.windows(2) {
            if pair[0].target == pair[1].target {
                return Err(EnvironmentPlanError::DuplicateTarget(
                    pair[0].target.clone(),
                ));
            }
        }

        normalize_warnings(&mut self.warnings, &self.targets)?;
        normalize_conflicts(&mut self.conflicts, &self.targets)
    }

    fn validate_provisioning_readiness(&self) -> Result<(), EnvironmentPlanError> {
        for target in &self.targets {
            validate_target_provisioning(target, &self.platforms)?;
        }
        Ok(())
    }

    fn unresolved_requirements(&self) -> usize {
        self.targets
            .iter()
            .map(|target| {
                target
                    .runtimes
                    .iter()
                    .filter(|runtime| !runtime.version.is_exact())
                    .count()
                    + target
                        .package_manager
                        .iter()
                        .filter(|manager| !manager.version.is_exact())
                        .count()
                    + target
                        .signal_tools
                        .iter()
                        .filter(|tool| !tool.version.is_exact())
                        .count()
            })
            .sum()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct ResolvedEnvironmentPlan(EnvironmentPlan);

impl ResolvedEnvironmentPlan {
    #[must_use]
    pub const fn plan(&self) -> &EnvironmentPlan {
        &self.0
    }

    #[must_use]
    pub fn into_plan(self) -> EnvironmentPlan {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvironmentPlanError {
    EmptyField(&'static str),
    NonPortablePath {
        field: &'static str,
        value: String,
    },
    PathOutsideOwner {
        field: &'static str,
        owner: String,
        path: String,
    },
    FloatingExactVersion(String),
    InvalidDigest {
        field: &'static str,
        value: String,
    },
    MissingPlatforms,
    MissingTargets,
    MissingRuntime(TargetIdentity),
    UnsupportedProvisioning {
        target: TargetIdentity,
        item: String,
    },
    UnsupportedPlatform {
        target: TargetIdentity,
        item: String,
    },
    CheckoutMutation {
        target: TargetIdentity,
        tool: String,
    },
    UnknownDiagnosticTarget(TargetIdentity),
    DuplicateTarget(TargetIdentity),
    BlockingConflicts(usize),
    UnresolvedRequirements(usize),
}

impl fmt::Display for EnvironmentPlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for EnvironmentPlanError {}

fn normalize_warnings(
    warnings: &mut Vec<EnvironmentWarning>,
    targets: &[TargetEnvironment],
) -> Result<(), EnvironmentPlanError> {
    for warning in warnings.iter_mut() {
        warning.code = required_label("warning code", warning.code.clone())?;
        warning.message = required_label("warning message", warning.message.clone())?;
        normalize_diagnostic_target(&mut warning.target, targets)?;
    }
    warnings.sort();
    warnings.dedup();
    Ok(())
}

fn normalize_conflicts(
    conflicts: &mut Vec<EnvironmentConflict>,
    targets: &[TargetEnvironment],
) -> Result<(), EnvironmentPlanError> {
    for conflict in conflicts.iter_mut() {
        conflict.code = required_label("conflict code", conflict.code.clone())?;
        conflict.message = required_label("conflict message", conflict.message.clone())?;
        normalize_diagnostic_target(&mut conflict.target, targets)?;
        for source in &mut conflict.sources {
            normalize_source(source)?;
        }
        conflict.sources.sort();
        conflict.sources.dedup();
    }
    conflicts.sort();
    conflicts.dedup();
    Ok(())
}

fn normalize_diagnostic_target(
    target: &mut Option<TargetIdentity>,
    targets: &[TargetEnvironment],
) -> Result<(), EnvironmentPlanError> {
    let Some(identity) = target else {
        return Ok(());
    };
    identity.root = normalize_repository_path("diagnostic target root", &identity.root)?;
    if targets.iter().any(|target| target.target == *identity) {
        Ok(())
    } else {
        Err(EnvironmentPlanError::UnknownDiagnosticTarget(
            identity.clone(),
        ))
    }
}

fn normalize_target_environment(
    target: &mut TargetEnvironment,
) -> Result<(), EnvironmentPlanError> {
    normalize_target_context(target)?;
    normalize_runtime_requirements(&target.target, &mut target.runtimes)?;
    if let Some(manager) = &mut target.package_manager {
        normalize_package_manager(manager)?;
    }
    normalize_signal_tools(&mut target.signal_tools)?;
    normalize_system_requirements(&mut target.system_requirements)?;
    normalize_dependency_locks(&mut target.dependency_locks)?;
    validate_target_ownership(target)
}

fn normalize_target_context(target: &mut TargetEnvironment) -> Result<(), EnvironmentPlanError> {
    target.target.root = normalize_repository_path("target root", &target.target.root)?;
    target.workspace = target
        .workspace
        .as_deref()
        .map(|path| normalize_repository_path("workspace root", path))
        .transpose()?;
    target.package = normalize_optional_label("package", target.package.take())?;
    Ok(())
}

fn normalize_runtime_requirements(
    target: &TargetIdentity,
    runtimes: &mut Vec<RuntimeRequirement>,
) -> Result<(), EnvironmentPlanError> {
    if runtimes.is_empty() {
        return Err(EnvironmentPlanError::MissingRuntime(target.clone()));
    }
    for runtime in runtimes.iter_mut() {
        runtime.runtime = required_label("runtime", runtime.runtime.clone())?;
        validate_version_requirement(&runtime.version)?;
        normalize_string_set("runtime component", &mut runtime.components)?;
        normalize_string_set("runtime target", &mut runtime.targets)?;
        normalize_source(&mut runtime.source)?;
    }
    runtimes.sort();
    runtimes.dedup();
    Ok(())
}

fn normalize_package_manager(
    manager: &mut PackageManagerRequirement,
) -> Result<(), EnvironmentPlanError> {
    manager.family = required_label("package manager", manager.family.clone())?;
    validate_version_requirement(&manager.version)?;
    manager.ownership_root =
        normalize_repository_path("package-manager ownership root", &manager.ownership_root)?;
    normalize_source(&mut manager.source)
}

fn normalize_signal_tools(
    signal_tools: &mut Vec<SignalToolRequirement>,
) -> Result<(), EnvironmentPlanError> {
    for tool in signal_tools.iter_mut() {
        tool.tool = required_label("signal tool", tool.tool.clone())?;
        validate_version_requirement(&tool.version)?;
        tool.provider = required_label("signal-tool provider", tool.provider.clone())?;
        tool.signals.sort();
        tool.signals.dedup();
        tool.supported_platforms.sort();
        tool.supported_platforms.dedup();
        normalize_source(&mut tool.source)?;
    }
    signal_tools.sort();
    signal_tools.dedup();
    Ok(())
}

fn normalize_system_requirements(
    requirements: &mut Vec<SystemRequirement>,
) -> Result<(), EnvironmentPlanError> {
    for requirement in requirements.iter_mut() {
        requirement.name = required_label("system requirement", requirement.name.clone())?;
        requirement.supported_platforms.sort();
        requirement.supported_platforms.dedup();
        normalize_source(&mut requirement.source)?;
    }
    requirements.sort();
    requirements.dedup();
    Ok(())
}

fn normalize_dependency_locks(
    dependency_locks: &mut Vec<DependencyLockRequirement>,
) -> Result<(), EnvironmentPlanError> {
    for dependency_lock in dependency_locks.iter_mut() {
        dependency_lock.path = normalize_repository_path("dependency lock", &dependency_lock.path)?;
        dependency_lock.owner_root =
            normalize_repository_path("dependency-lock owner", &dependency_lock.owner_root)?;
        dependency_lock.digest =
            validate_digest("dependency-lock digest", dependency_lock.digest.clone())?;
        normalize_source(&mut dependency_lock.source)?;
    }
    dependency_locks.sort();
    dependency_locks.dedup();
    Ok(())
}

fn validate_target_provisioning(
    target: &TargetEnvironment,
    requested_platforms: &[TargetPlatform],
) -> Result<(), EnvironmentPlanError> {
    for tool in &target.signal_tools {
        validate_requirement_platforms(
            &target.target,
            &tool.tool,
            &tool.supported_platforms,
            requested_platforms,
        )?;
        if tool.provisioning == ProvisioningSupport::Unsupported {
            return Err(EnvironmentPlanError::UnsupportedProvisioning {
                target: target.target.clone(),
                item: tool.tool.clone(),
            });
        }
        if tool.scope == ToolInstallationScope::Project && tool.modifies_checkout {
            return Err(EnvironmentPlanError::CheckoutMutation {
                target: target.target.clone(),
                tool: tool.tool.clone(),
            });
        }
    }
    for requirement in &target.system_requirements {
        validate_requirement_platforms(
            &target.target,
            &requirement.name,
            &requirement.supported_platforms,
            requested_platforms,
        )?;
        if requirement.provisioning == ProvisioningSupport::Unsupported {
            return Err(EnvironmentPlanError::UnsupportedProvisioning {
                target: target.target.clone(),
                item: requirement.name.clone(),
            });
        }
    }
    Ok(())
}

fn validate_requirement_platforms(
    target: &TargetIdentity,
    item: &str,
    supported: &[TargetPlatform],
    requested: &[TargetPlatform],
) -> Result<(), EnvironmentPlanError> {
    if supported.is_empty()
        || requested
            .iter()
            .all(|platform| supported.contains(platform))
    {
        Ok(())
    } else {
        Err(EnvironmentPlanError::UnsupportedPlatform {
            target: target.clone(),
            item: item.to_string(),
        })
    }
}

fn validate_target_ownership(target: &TargetEnvironment) -> Result<(), EnvironmentPlanError> {
    if let Some(workspace) = &target.workspace {
        ensure_contains("workspace root", workspace, &target.target.root)?;
    }
    if let Some(manager) = &target.package_manager {
        ensure_contains(
            "package-manager ownership root",
            &manager.ownership_root,
            &target.target.root,
        )?;
    }
    for dependency_lock in &target.dependency_locks {
        ensure_contains(
            "dependency lock",
            &dependency_lock.owner_root,
            &dependency_lock.path,
        )?;
        ensure_contains(
            "dependency-lock owner",
            &dependency_lock.owner_root,
            &target.target.root,
        )?;
    }
    Ok(())
}

fn ensure_contains(
    field: &'static str,
    owner: &str,
    path: &str,
) -> Result<(), EnvironmentPlanError> {
    let owner = repository_components(owner);
    let path_components = repository_components(path);
    if path_components.starts_with(&owner) {
        Ok(())
    } else {
        Err(EnvironmentPlanError::PathOutsideOwner {
            field,
            owner: owner.join("/"),
            path: path.to_string(),
        })
    }
}

fn repository_components(path: &str) -> Vec<&str> {
    if path == "." {
        Vec::new()
    } else {
        path.split('/').collect()
    }
}

fn validate_version_requirement(
    requirement: &VersionRequirement,
) -> Result<(), EnvironmentPlanError> {
    match requirement {
        VersionRequirement::Exact { version } => {
            let version = required_label("exact version", version.clone())?;
            reject_floating_version(&version)
        }
        VersionRequirement::Selector { expression } => {
            required_label("version selector", expression.clone()).map(drop)
        }
        VersionRequirement::Compatibility { expression } => {
            required_label("version compatibility", expression.clone()).map(drop)
        }
        VersionRequirement::Minimum { version } => {
            required_label("minimum version", version.clone()).map(drop)
        }
        VersionRequirement::Unresolved { reason } => {
            required_label("unresolved reason", reason.clone()).map(drop)
        }
    }
}

fn normalize_source(source: &mut RequirementSource) -> Result<(), EnvironmentPlanError> {
    source.kind = required_label("source kind", source.kind.clone())?;
    source.path = normalize_repository_path("requirement source", &source.path)?;
    source.detail = normalize_optional_label("source detail", source.detail.take())?;
    Ok(())
}

fn normalize_string_set(
    field: &'static str,
    values: &mut Vec<String>,
) -> Result<(), EnvironmentPlanError> {
    for value in values.iter_mut() {
        *value = required_label(field, value.clone())?;
    }
    values.sort();
    values.dedup();
    Ok(())
}

fn normalize_optional_label(
    field: &'static str,
    value: Option<String>,
) -> Result<Option<String>, EnvironmentPlanError> {
    value.map(|value| required_label(field, value)).transpose()
}

fn required_label(field: &'static str, value: String) -> Result<String, EnvironmentPlanError> {
    let value = value.trim().to_string();
    if value.is_empty() {
        Err(EnvironmentPlanError::EmptyField(field))
    } else {
        Ok(value)
    }
}

fn normalize_repository_path(
    field: &'static str,
    value: &str,
) -> Result<String, EnvironmentPlanError> {
    let portable = value.trim().replace('\\', "/");
    let has_windows_prefix = portable.as_bytes().get(1) == Some(&b':')
        && portable
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphabetic);
    if portable.starts_with('/') || has_windows_prefix || Path::new(&portable).is_absolute() {
        return Err(EnvironmentPlanError::NonPortablePath {
            field,
            value: value.to_string(),
        });
    }

    let mut normalized = PathBuf::new();
    for component in Path::new(&portable).components() {
        match component {
            Component::Normal(component) => normalized.push(component),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(EnvironmentPlanError::NonPortablePath {
                    field,
                    value: value.to_string(),
                });
            }
        }
    }
    if normalized.as_os_str().is_empty() {
        Ok(String::from("."))
    } else {
        Ok(normalized.to_string_lossy().replace('\\', "/"))
    }
}

fn reject_floating_version(version: &str) -> Result<(), EnvironmentPlanError> {
    if matches!(
        version.to_ascii_lowercase().as_str(),
        "latest" | "stable" | "*"
    ) {
        Err(EnvironmentPlanError::FloatingExactVersion(
            version.to_string(),
        ))
    } else {
        Ok(())
    }
}

fn validate_digest(field: &'static str, value: String) -> Result<String, EnvironmentPlanError> {
    let value = value.trim().to_ascii_lowercase();
    let digest = value.strip_prefix("sha256:").unwrap_or(&value);
    if digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(format!("sha256:{digest}"))
    } else {
        Err(EnvironmentPlanError::InvalidDigest { field, value })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest(character: char) -> String {
        format!("sha256:{}", character.to_string().repeat(64))
    }

    fn source(path: &str) -> RequirementSource {
        RequirementSource::new(
            "manifest",
            path,
            None::<String>,
            RequirementConfidence::Declared,
        )
        .expect("source")
    }

    fn platform() -> TargetPlatform {
        TargetPlatform {
            os: OperatingSystem::Linux,
            architecture: Architecture::Amd64,
            libc: Libc::Glibc,
        }
    }

    fn target(root: &str, runtime_version: VersionRequirement) -> TargetEnvironment {
        TargetEnvironment {
            target: TargetIdentity::new(Language::Node, root).expect("target"),
            workspace: Some(root.to_string()),
            package: Some(String::from("@example/web")),
            runtimes: vec![RuntimeRequirement {
                runtime: String::from("node"),
                version: runtime_version,
                components: vec![String::from("corepack"), String::from("corepack")],
                targets: Vec::new(),
                source: source(&format!("{root}/package.json")),
            }],
            package_manager: Some(PackageManagerRequirement {
                family: String::from("pnpm"),
                version: VersionRequirement::exact("10.14.0").expect("version"),
                ownership_root: root.to_string(),
                source: source(&format!("{root}/package.json")),
            }),
            signal_tools: vec![SignalToolRequirement {
                tool: String::from("vitest"),
                version: VersionRequirement::exact("3.2.4").expect("version"),
                provider: String::from("project_dependency"),
                scope: ToolInstallationScope::Project,
                signals: vec![SignalKind::Coverage, SignalKind::Test, SignalKind::Test],
                supported_platforms: vec![platform()],
                provisioning: ProvisioningSupport::LockedOffline,
                modifies_checkout: false,
                source: source(&format!("{root}/package.json")),
            }],
            system_requirements: vec![SystemRequirement {
                kind: SystemRequirementKind::Capability,
                name: String::from("native-build"),
                supported_platforms: vec![platform()],
                provisioning: ProvisioningSupport::LockedOffline,
                source: source(&format!("{root}/package.json")),
            }],
            dependency_locks: vec![DependencyLockRequirement {
                path: format!("{root}/pnpm-lock.yaml"),
                digest: digest('a'),
                owner_root: root.to_string(),
                source: source(&format!("{root}/pnpm-lock.yaml")),
            }],
        }
    }

    fn plan(targets: Vec<TargetEnvironment>) -> EnvironmentPlan {
        EnvironmentPlan::new(
            RepositoryIdentity {
                name: String::from("fixture"),
                contract_digest: digest('b'),
            },
            vec![platform(), platform()],
            targets,
            Vec::new(),
            Vec::new(),
        )
        .expect("plan")
    }

    #[test]
    fn normalized_target_identity_is_checkout_independent() {
        let target = TargetIdentity::new(Language::Rust, r".\crates\core\").expect("identity");
        assert_eq!(target.root, "crates/core");
    }

    #[test]
    fn equal_semantic_inputs_produce_byte_stable_ordered_plans() {
        let first = plan(vec![
            target(
                "apps/zeta",
                VersionRequirement::exact("22.1.0").expect("version"),
            ),
            target(
                "apps/alpha",
                VersionRequirement::exact("20.2.0").expect("version"),
            ),
        ]);
        let second = plan(vec![
            target(
                "apps/alpha",
                VersionRequirement::exact("20.2.0").expect("version"),
            ),
            target(
                "apps/zeta",
                VersionRequirement::exact("22.1.0").expect("version"),
            ),
        ]);
        assert_eq!(first, second);
        assert_eq!(
            serde_json::to_string(&first).expect("serialize"),
            serde_json::to_string(&second).expect("serialize")
        );
        assert_eq!(first.targets()[0].target.root, "apps/alpha");
        assert_eq!(first.targets()[0].runtimes[0].components, ["corepack"]);
        assert_eq!(
            first.targets()[0].signal_tools[0].signals,
            [SignalKind::Test, SignalKind::Coverage]
        );
    }

    #[test]
    fn version_evidence_preserves_ecosystem_semantics() {
        let values = [
            VersionRequirement::selector("stable").expect("selector"),
            VersionRequirement::compatibility(">=20 <23").expect("compatibility"),
            VersionRequirement::minimum("1.80").expect("minimum"),
        ];
        let json = serde_json::to_value(values).expect("serialize");
        assert_eq!(json[0]["state"], "selector");
        assert_eq!(json[1]["state"], "compatibility");
        assert_eq!(json[2]["state"], "minimum");
    }

    #[test]
    fn workspace_and_package_do_not_change_target_identity() {
        let mut first = target(
            "apps/web",
            VersionRequirement::exact("22.1.0").expect("version"),
        );
        let mut second = first.clone();
        first.workspace = Some(String::from("."));
        second.workspace = Some(String::from("apps"));
        second.package = Some(String::from("renamed-package"));
        assert_eq!(first.target, second.target);
        assert!(matches!(
            EnvironmentPlan::new(
                RepositoryIdentity {
                    name: String::from("fixture"),
                    contract_digest: digest('b'),
                },
                vec![platform()],
                vec![first, second],
                Vec::new(),
                Vec::new(),
            ),
            Err(EnvironmentPlanError::DuplicateTarget(_))
        ));
    }

    #[test]
    fn conflicts_and_unresolved_requirements_cannot_be_resolved() {
        let conflict = EnvironmentConflict {
            code: String::from("runtime_conflict"),
            message: String::from("runtime sources disagree"),
            target: None,
            sources: vec![source("rust-toolchain.toml")],
        };
        let conflicting = EnvironmentPlan::new(
            RepositoryIdentity {
                name: String::from("fixture"),
                contract_digest: digest('b'),
            },
            vec![platform()],
            vec![target(
                "apps/web",
                VersionRequirement::exact("22.1.0").expect("version"),
            )],
            Vec::new(),
            vec![conflict],
        )
        .expect("plan");
        assert_eq!(
            conflicting.resolve(),
            Err(EnvironmentPlanError::BlockingConflicts(1))
        );

        let unresolved = plan(vec![target(
            "apps/web",
            VersionRequirement::compatibility(">=20 <23").expect("compatibility"),
        )]);
        assert_eq!(
            unresolved.resolve(),
            Err(EnvironmentPlanError::UnresolvedRequirements(1))
        );
    }

    #[test]
    fn resolved_plan_requires_exact_non_floating_versions() {
        assert_eq!(
            VersionRequirement::exact("latest"),
            Err(EnvironmentPlanError::FloatingExactVersion(String::from(
                "latest"
            )))
        );
        let resolved = plan(vec![target(
            "apps/web",
            VersionRequirement::exact("22.1.0").expect("version"),
        )])
        .resolve()
        .expect("resolved plan");
        assert!(resolved.plan().targets()[0].runtimes[0].version.is_exact());
    }

    #[test]
    fn absolute_parent_and_windows_paths_are_rejected() {
        for path in ["/tmp/repo", "../outside", r"C:\\repo", "apps/../../outside"] {
            let error = TargetIdentity::new(Language::Rust, path).expect_err("path must fail");
            assert!(matches!(
                error,
                EnvironmentPlanError::NonPortablePath { .. }
            ));
        }
    }

    #[test]
    fn duplicate_targets_and_invalid_digests_fail_validation() {
        let duplicate = target(
            "apps/web",
            VersionRequirement::exact("22.1.0").expect("version"),
        );
        assert!(matches!(
            EnvironmentPlan::new(
                RepositoryIdentity {
                    name: String::from("fixture"),
                    contract_digest: digest('b'),
                },
                vec![platform()],
                vec![duplicate.clone(), duplicate],
                Vec::new(),
                Vec::new(),
            ),
            Err(EnvironmentPlanError::DuplicateTarget(_))
        ));
        assert!(matches!(
            EnvironmentPlan::new(
                RepositoryIdentity {
                    name: String::from("fixture"),
                    contract_digest: String::from("not-a-digest"),
                },
                vec![platform()],
                Vec::new(),
                Vec::new(),
                Vec::new(),
            ),
            Err(EnvironmentPlanError::InvalidDigest { .. })
        ));
    }

    #[test]
    fn diagnostic_paths_are_normalized_and_must_reference_plan_targets() {
        let identity = TargetIdentity::new(Language::Node, "apps/web").expect("target");
        let warning = EnvironmentWarning {
            code: String::from("missing_pin"),
            message: String::from("runtime is not pinned"),
            target: Some(TargetIdentity {
                language: Language::Node,
                root: String::from("apps\\web"),
            }),
        };
        let normalized = EnvironmentPlan::new(
            RepositoryIdentity {
                name: String::from("fixture"),
                contract_digest: digest('b'),
            },
            vec![platform()],
            vec![target(
                "apps/web",
                VersionRequirement::exact("22.1.0").expect("version"),
            )],
            vec![warning],
            Vec::new(),
        )
        .expect("plan");
        assert_eq!(normalized.warnings()[0].target.as_ref(), Some(&identity));

        let unknown = EnvironmentWarning {
            code: String::from("missing_pin"),
            message: String::from("runtime is not pinned"),
            target: Some(TargetIdentity {
                language: Language::Node,
                root: String::from("apps/other"),
            }),
        };
        assert!(matches!(
            EnvironmentPlan::new(
                RepositoryIdentity {
                    name: String::from("fixture"),
                    contract_digest: digest('b'),
                },
                vec![platform()],
                vec![target(
                    "apps/web",
                    VersionRequirement::exact("22.1.0").expect("version"),
                )],
                vec![unknown],
                Vec::new(),
            ),
            Err(EnvironmentPlanError::UnknownDiagnosticTarget(_))
        ));
    }

    #[test]
    fn conflict_sources_cannot_contain_host_paths() {
        let conflict = EnvironmentConflict {
            code: String::from("runtime_conflict"),
            message: String::from("runtime sources disagree"),
            target: None,
            sources: vec![RequirementSource {
                kind: String::from("manifest"),
                path: String::from("/tmp/Cargo.toml"),
                detail: None,
                confidence: RequirementConfidence::Declared,
            }],
        };
        assert!(matches!(
            EnvironmentPlan::new(
                RepositoryIdentity {
                    name: String::from("fixture"),
                    contract_digest: digest('b'),
                },
                vec![platform()],
                vec![target(
                    "apps/web",
                    VersionRequirement::exact("22.1.0").expect("version"),
                )],
                Vec::new(),
                vec![conflict],
            ),
            Err(EnvironmentPlanError::NonPortablePath { .. })
        ));
    }

    #[test]
    fn normalization_rejects_forged_floating_versions() {
        let mut target = target(
            "apps/web",
            VersionRequirement::exact("22.1.0").expect("version"),
        );
        target.runtimes[0].version = VersionRequirement::Exact {
            version: String::from("latest"),
        };
        assert_eq!(
            EnvironmentPlan::new(
                RepositoryIdentity {
                    name: String::from("fixture"),
                    contract_digest: digest('b'),
                },
                vec![platform()],
                vec![target],
                Vec::new(),
                Vec::new(),
            ),
            Err(EnvironmentPlanError::FloatingExactVersion(String::from(
                "latest"
            )))
        );
    }

    #[test]
    fn ownership_uses_path_components_not_string_prefixes() {
        let mut target = target(
            "apps/api-v2",
            VersionRequirement::exact("22.1.0").expect("version"),
        );
        target.workspace = Some(String::from("apps/api"));
        assert!(matches!(
            EnvironmentPlan::new(
                RepositoryIdentity {
                    name: String::from("fixture"),
                    contract_digest: digest('b'),
                },
                vec![platform()],
                vec![target],
                Vec::new(),
                Vec::new(),
            ),
            Err(EnvironmentPlanError::PathOutsideOwner { .. })
        ));
    }

    #[test]
    fn dependency_locks_must_be_below_their_owner() {
        let mut target = target(
            "apps/web",
            VersionRequirement::exact("22.1.0").expect("version"),
        );
        target.dependency_locks[0].path = String::from("shared/pnpm-lock.yaml");
        assert!(matches!(
            EnvironmentPlan::new(
                RepositoryIdentity {
                    name: String::from("fixture"),
                    contract_digest: digest('b'),
                },
                vec![platform()],
                vec![target],
                Vec::new(),
                Vec::new(),
            ),
            Err(EnvironmentPlanError::PathOutsideOwner { .. })
        ));
    }

    #[test]
    fn deserialization_rejects_invalid_schema_paths_and_digests() {
        let valid = serde_json::to_value(plan(vec![target(
            "apps/web",
            VersionRequirement::exact("22.1.0").expect("version"),
        )]))
        .expect("serialize");
        for (pointer, invalid) in [
            ("/schema_version", serde_json::json!("9.9.9")),
            ("/targets/0/target/root", serde_json::json!("/tmp/repo")),
            ("/repository/contract_digest", serde_json::json!("bad")),
        ] {
            let mut value = valid.clone();
            *value.pointer_mut(pointer).expect("pointer") = invalid;
            assert!(
                serde_json::from_value::<EnvironmentPlan>(value).is_err(),
                "{pointer}"
            );
        }
    }

    #[test]
    fn zero_target_plan_fails_closed() {
        assert_eq!(
            EnvironmentPlan::new(
                RepositoryIdentity {
                    name: String::from("fixture"),
                    contract_digest: digest('b'),
                },
                vec![platform()],
                Vec::new(),
                Vec::new(),
                Vec::new(),
            ),
            Err(EnvironmentPlanError::MissingTargets)
        );
    }

    #[test]
    fn provisioning_readiness_rejects_unsupported_or_mutating_requirements() {
        let mut unsupported = target(
            "apps/web",
            VersionRequirement::exact("22.1.0").expect("version"),
        );
        unsupported.system_requirements[0].provisioning = ProvisioningSupport::Unsupported;
        assert!(matches!(
            plan(vec![unsupported]).resolve(),
            Err(EnvironmentPlanError::UnsupportedProvisioning { .. })
        ));

        let mut mutating = target(
            "apps/web",
            VersionRequirement::exact("22.1.0").expect("version"),
        );
        mutating.signal_tools[0].modifies_checkout = true;
        assert!(matches!(
            plan(vec![mutating]).resolve(),
            Err(EnvironmentPlanError::CheckoutMutation { .. })
        ));
    }

    #[test]
    fn provisioning_readiness_rejects_unsupported_requested_platform() {
        let mut target = target(
            "apps/web",
            VersionRequirement::exact("22.1.0").expect("version"),
        );
        target.signal_tools[0].supported_platforms = vec![TargetPlatform {
            os: OperatingSystem::Linux,
            architecture: Architecture::Arm64,
            libc: Libc::Glibc,
        }];
        assert!(matches!(
            plan(vec![target]).resolve(),
            Err(EnvironmentPlanError::UnsupportedPlatform { .. })
        ));
    }

    #[test]
    fn targets_require_runtime_evidence() {
        let mut target = target(
            "apps/web",
            VersionRequirement::exact("22.1.0").expect("version"),
        );
        target.runtimes.clear();
        assert!(matches!(
            EnvironmentPlan::new(
                RepositoryIdentity {
                    name: String::from("fixture"),
                    contract_digest: digest('b'),
                },
                vec![platform()],
                vec![target],
                Vec::new(),
                Vec::new(),
            ),
            Err(EnvironmentPlanError::MissingRuntime(_))
        ));
    }

    #[test]
    fn serialized_contract_contains_no_provider_commands_or_host_paths() {
        let json = serde_json::to_string_pretty(&plan(vec![target(
            "apps/web",
            VersionRequirement::exact("22.1.0").expect("version"),
        )]))
        .expect("serialize");
        assert!(!json.contains("mise"));
        assert!(!json.contains("Dockerfile"));
        assert!(!json.contains("/Users/"));
        assert!(!json.contains("command"));
    }
}
