//! Read-only Node environment discovery.

use crate::package_manager::PackageManager;
use ayni_adapters_common::repository::{
    read_contained_string, read_optional_contained_bytes, read_optional_contained_string,
    repository_relative,
};
use ayni_core::{
    AdapterError, DependencyLockRequirement, EnvironmentCapability, EnvironmentConflict,
    EnvironmentContribution, EnvironmentDiscoveryRequest, EnvironmentWarning, Language,
    PackageManagerRequirement, ProvisioningSupport, RequirementConfidence, RequirementSource,
    RuntimeRequirement, SignalKind, SignalToolRequirement, TargetEnvironment,
    ToolInstallationScope, VersionRequirement,
};
use glob::Pattern;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Default)]
pub(crate) struct NodeEnvironmentCapability;

type PackageManagerDiscovery = (
    PackageManagerRequirement,
    Vec<DependencyLockRequirement>,
    Vec<EnvironmentConflict>,
    Vec<EnvironmentWarning>,
);

impl EnvironmentCapability for NodeEnvironmentCapability {
    fn language(&self) -> Language {
        Language::Node
    }

    fn discover(
        &self,
        request: &EnvironmentDiscoveryRequest,
    ) -> Result<EnvironmentContribution, AdapterError> {
        discover(request)
    }
}

fn discover(
    request: &EnvironmentDiscoveryRequest,
) -> Result<EnvironmentContribution, AdapterError> {
    let target_root = request.target_root();
    let target_manifest =
        read_manifest(request.repo_root(), &target_root.join("package.json"), true)?
            .expect("required manifest");
    let workspace_root = workspace_owner(request.repo_root(), &target_root)?;
    let workspace_manifest = if workspace_root == target_root {
        target_manifest.clone()
    } else {
        read_manifest(
            request.repo_root(),
            &workspace_root.join("package.json"),
            true,
        )?
        .expect("workspace manifest")
    };

    let (runtime, mut conflicts) = runtime_requirement(
        request,
        &target_root,
        &target_manifest,
        &workspace_root,
        &workspace_manifest,
    )?;
    let manager_root = package_manager_owner(
        request.repo_root(),
        &target_root,
        &target_manifest,
        &workspace_root,
    )?;
    let manager_manifest = if manager_root == target_root {
        &target_manifest
    } else {
        &workspace_manifest
    };
    let (package_manager, locks, manager_conflicts, mut warnings) =
        package_manager_requirement(request, &manager_root, manager_manifest)?;
    conflicts.extend(manager_conflicts);

    if matches!(runtime.version, VersionRequirement::Unresolved { .. }) {
        warnings.push(EnvironmentWarning {
            code: String::from("node_runtime_unresolved"),
            message: String::from(
                "No Node selector or engines.node declaration was found; add .node-version, .nvmrc, or package.json engines.node.",
            ),
            target: Some(request.target().clone()),
        });
    }

    EnvironmentContribution::new(
        TargetEnvironment {
            target: request.target().clone(),
            workspace: (workspace_root != target_root)
                .then(|| relative(request.repo_root(), &workspace_root))
                .transpose()?,
            package: target_manifest
                .get("name")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned),
            runtimes: vec![runtime],
            package_manager: Some(package_manager),
            signal_tools: signal_tools(
                request,
                &target_root,
                &target_manifest,
                &manager_root,
                manager_manifest,
            )?,
            system_requirements: Vec::new(),
            dependency_locks: locks,
        },
        warnings,
        conflicts,
    )
    .map_err(adapter_error)
}

fn runtime_requirement(
    request: &EnvironmentDiscoveryRequest,
    target_root: &Path,
    target_manifest: &serde_json::Value,
    workspace_root: &Path,
    workspace_manifest: &serde_json::Value,
) -> Result<(RuntimeRequirement, Vec<EnvironmentConflict>), AdapterError> {
    let selectors = selected_runtime_selectors(request.repo_root(), target_root, workspace_root)?;
    let (version, source, conflicts) =
        if let Some(selection) = selector_runtime(request, &selectors)? {
            selection
        } else {
            let (version, source) = manifest_runtime(
                request,
                target_root,
                target_manifest,
                workspace_root,
                workspace_manifest,
            )?;
            (version, source, Vec::new())
        };

    Ok((
        RuntimeRequirement {
            runtime: String::from("node"),
            version,
            components: Vec::new(),
            targets: Vec::new(),
            source,
        },
        conflicts,
    ))
}

fn selected_runtime_selectors(
    repo_root: &Path,
    target_root: &Path,
    workspace_root: &Path,
) -> Result<Vec<SelectorEvidence>, AdapterError> {
    let target = selector_files(repo_root, target_root)?;
    let workspace = if workspace_root == target_root {
        Vec::new()
    } else {
        selector_files(repo_root, workspace_root)?
    };
    Ok(if target.is_empty() { workspace } else { target })
}

fn selector_runtime(
    request: &EnvironmentDiscoveryRequest,
    selectors: &[SelectorEvidence],
) -> Result<
    Option<(
        VersionRequirement,
        RequirementSource,
        Vec<EnvironmentConflict>,
    )>,
    AdapterError,
> {
    let Some(first) = selectors.first() else {
        return Ok(None);
    };
    let values = selectors
        .iter()
        .map(|selector| selector.value.as_str())
        .collect::<BTreeSet<_>>();
    if values.len() == 1 {
        return Ok(Some((
            selector_requirement(&first.value)?,
            first.source.clone(),
            Vec::new(),
        )));
    }

    let conflict = EnvironmentConflict {
        code: String::from("node_runtime_selector_conflict"),
        message: format!(
            "Node runtime selectors disagree: {}",
            values.into_iter().collect::<Vec<_>>().join(", ")
        ),
        target: Some(request.target().clone()),
        sources: selectors
            .iter()
            .map(|selector| selector.source.clone())
            .collect(),
    };
    Ok(Some((
        VersionRequirement::unresolved("conflicting Node runtime selector files")
            .map_err(adapter_error)?,
        first.source.clone(),
        vec![conflict],
    )))
}

fn manifest_runtime(
    request: &EnvironmentDiscoveryRequest,
    target_root: &Path,
    target_manifest: &serde_json::Value,
    workspace_root: &Path,
    workspace_manifest: &serde_json::Value,
) -> Result<(VersionRequirement, RequirementSource), AdapterError> {
    if let Some(expression) = engines_node(target_manifest)? {
        return declared_manifest_runtime(request, target_root, expression);
    }
    if workspace_root != target_root
        && let Some(expression) = engines_node(workspace_manifest)?
    {
        return declared_manifest_runtime(request, workspace_root, expression);
    }
    Ok((
        VersionRequirement::unresolved("no Node runtime selector or engines.node declaration")
            .map_err(adapter_error)?,
        source(
            request.repo_root(),
            &target_root.join("package.json"),
            "node_runtime_unresolved",
            None,
            RequirementConfidence::Assumed,
        )?,
    ))
}

fn declared_manifest_runtime(
    request: &EnvironmentDiscoveryRequest,
    root: &Path,
    expression: &str,
) -> Result<(VersionRequirement, RequirementSource), AdapterError> {
    Ok((
        VersionRequirement::compatibility(expression).map_err(adapter_error)?,
        source(
            request.repo_root(),
            &root.join("package.json"),
            "package_json_engines_node",
            Some(expression),
            RequirementConfidence::Declared,
        )?,
    ))
}

#[derive(Debug)]
struct SelectorEvidence {
    value: String,
    source: RequirementSource,
}

fn selector_files(repo_root: &Path, root: &Path) -> Result<Vec<SelectorEvidence>, AdapterError> {
    let mut selectors = Vec::new();
    for name in [".node-version", ".nvmrc"] {
        let path = root.join(name);
        let Some(content) =
            read_optional_contained_string(repo_root, &path).map_err(adapter_error)?
        else {
            continue;
        };
        let value = content.trim();
        if value.is_empty() || value.lines().count() != 1 {
            return Err(adapter_error(format!(
                "{} must contain one non-empty Node selector",
                path.display()
            )));
        }
        selectors.push(SelectorEvidence {
            value: value.to_string(),
            source: source(
                repo_root,
                &path,
                "node_runtime_selector",
                Some(value),
                RequirementConfidence::Declared,
            )?,
        });
    }
    Ok(selectors)
}

fn engines_node(manifest: &serde_json::Value) -> Result<Option<&str>, AdapterError> {
    let Some(engines) = manifest.get("engines") else {
        return Ok(None);
    };
    let engines = engines
        .as_object()
        .ok_or_else(|| adapter_error("package.json engines must be an object"))?;
    let Some(node) = engines.get("node") else {
        return Ok(None);
    };
    node.as_str()
        .map(Some)
        .ok_or_else(|| adapter_error("package.json engines.node must be a string"))
}

fn package_manager_owner(
    repo_root: &Path,
    target_root: &Path,
    target_manifest: &serde_json::Value,
    workspace_root: &Path,
) -> Result<PathBuf, AdapterError> {
    if target_manifest.get("packageManager").is_some()
        || !lock_paths(repo_root, target_root)?.is_empty()
    {
        Ok(target_root.to_path_buf())
    } else {
        Ok(workspace_root.to_path_buf())
    }
}

fn package_manager_requirement(
    request: &EnvironmentDiscoveryRequest,
    owner: &Path,
    manifest: &serde_json::Value,
) -> Result<PackageManagerDiscovery, AdapterError> {
    let locks = dependency_locks(request.repo_root(), owner)?;
    let families = lock_families(&locks);
    let declared = declared_package_manager(manifest)?;
    let conflicts = package_manager_conflicts(request, owner, &locks, &families, &declared)?;
    let (requirement, warnings) =
        select_package_manager(request, owner, &locks, &families, declared)?;

    Ok((requirement, locks, conflicts, warnings))
}

type DeclaredPackageManager<'a> = (String, VersionRequirement, &'a str);

fn lock_families(locks: &[DependencyLockRequirement]) -> BTreeSet<&'static str> {
    locks
        .iter()
        .filter_map(|lock| lock_family(&lock.path))
        .collect()
}

fn declared_package_manager(
    manifest: &serde_json::Value,
) -> Result<Option<DeclaredPackageManager<'_>>, AdapterError> {
    manifest
        .get("packageManager")
        .map(|value| {
            let value = value
                .as_str()
                .ok_or_else(|| adapter_error("package.json packageManager must be a string"))?;
            parse_package_manager(value).map(|(family, version)| (family, version, value))
        })
        .transpose()
}

fn package_manager_conflicts(
    request: &EnvironmentDiscoveryRequest,
    owner: &Path,
    locks: &[DependencyLockRequirement],
    families: &BTreeSet<&str>,
    declared: &Option<DeclaredPackageManager<'_>>,
) -> Result<Vec<EnvironmentConflict>, AdapterError> {
    let mut conflicts = Vec::new();
    if locks.is_empty() {
        conflicts.push(missing_lock_conflict(request, owner)?);
    }
    if let Some((family, _, raw)) = declared
        && families.iter().any(|lock_family| lock_family != family)
    {
        conflicts.push(lock_declaration_conflict(
            request, owner, locks, families, family, raw,
        )?);
    }
    if declared.is_none() && families.len() > 1 {
        conflicts.push(EnvironmentConflict {
            code: String::from("node_package_manager_lock_ambiguity"),
            message: format!(
                "Multiple package-manager lockfile families are present: {}",
                families.iter().copied().collect::<Vec<_>>().join(", ")
            ),
            target: Some(request.target().clone()),
            sources: locks.iter().map(|lock| lock.source.clone()).collect(),
        });
    }
    Ok(conflicts)
}

fn missing_lock_conflict(
    request: &EnvironmentDiscoveryRequest,
    owner: &Path,
) -> Result<EnvironmentConflict, AdapterError> {
    Ok(EnvironmentConflict {
        code: String::from("node_dependency_lock_missing"),
        message: String::from(
            "No native Node dependency lockfile was found; commit the selected package manager's lockfile before locking the environment.",
        ),
        target: Some(request.target().clone()),
        sources: vec![source(
            request.repo_root(),
            &owner.join("package.json"),
            "node_dependency_lock_missing",
            None,
            RequirementConfidence::Assumed,
        )?],
    })
}

fn lock_declaration_conflict(
    request: &EnvironmentDiscoveryRequest,
    owner: &Path,
    locks: &[DependencyLockRequirement],
    families: &BTreeSet<&str>,
    family: &str,
    raw: &str,
) -> Result<EnvironmentConflict, AdapterError> {
    let mut sources = vec![source(
        request.repo_root(),
        &owner.join("package.json"),
        "package_json_package_manager",
        Some(raw),
        RequirementConfidence::Declared,
    )?];
    sources.extend(locks.iter().map(|lock| lock.source.clone()));
    Ok(EnvironmentConflict {
        code: String::from("node_package_manager_lock_conflict"),
        message: format!(
            "packageManager declares {family}, but lockfiles indicate {}",
            families.iter().copied().collect::<Vec<_>>().join(", ")
        ),
        target: Some(request.target().clone()),
        sources,
    })
}

fn select_package_manager(
    request: &EnvironmentDiscoveryRequest,
    owner: &Path,
    locks: &[DependencyLockRequirement],
    families: &BTreeSet<&str>,
    declared: Option<DeclaredPackageManager<'_>>,
) -> Result<(PackageManagerRequirement, Vec<EnvironmentWarning>), AdapterError> {
    if let Some((family, version, raw)) = declared {
        return Ok((
            PackageManagerRequirement {
                family,
                version,
                ownership_root: relative(request.repo_root(), owner)?,
                source: source(
                    request.repo_root(),
                    &owner.join("package.json"),
                    "package_json_package_manager",
                    Some(raw),
                    RequirementConfidence::Declared,
                )?,
            },
            Vec::new(),
        ));
    }
    if let Some(family) = families.iter().next() {
        return Ok((
            lock_package_manager(request, owner, locks, family)?,
            Vec::new(),
        ));
    }
    fallback_package_manager(request, owner)
}

fn lock_package_manager(
    request: &EnvironmentDiscoveryRequest,
    owner: &Path,
    locks: &[DependencyLockRequirement],
    family: &str,
) -> Result<PackageManagerRequirement, AdapterError> {
    Ok(PackageManagerRequirement {
        family: family.to_string(),
        version: VersionRequirement::unresolved(
            "native lockfile does not pin package-manager version",
        )
        .map_err(adapter_error)?,
        ownership_root: relative(request.repo_root(), owner)?,
        source: locks
            .iter()
            .find(|lock| lock_family(&lock.path) == Some(family))
            .expect("family lock")
            .source
            .clone(),
    })
}

fn fallback_package_manager(
    request: &EnvironmentDiscoveryRequest,
    owner: &Path,
) -> Result<(PackageManagerRequirement, Vec<EnvironmentWarning>), AdapterError> {
    Ok((
        PackageManagerRequirement {
            family: String::from("npm"),
            version: VersionRequirement::unresolved(
                "npm fallback version is not declared by repository evidence",
            )
            .map_err(adapter_error)?,
            ownership_root: relative(request.repo_root(), owner)?,
            source: source(
                request.repo_root(),
                &owner.join("package.json"),
                "node_package_manager_fallback",
                Some("npm"),
                RequirementConfidence::Assumed,
            )?,
        },
        vec![EnvironmentWarning {
            code: String::from("node_package_manager_unresolved"),
            message: String::from(
                "No packageManager declaration or native lockfile was found; npm is the execution fallback but its version is unresolved.",
            ),
            target: Some(request.target().clone()),
        }],
    ))
}

fn signal_tools(
    request: &EnvironmentDiscoveryRequest,
    target_root: &Path,
    target_manifest: &serde_json::Value,
    owner_root: &Path,
    owner_manifest: &serde_json::Value,
) -> Result<Vec<SignalToolRequirement>, AdapterError> {
    const TOOLS: &[(&str, &[SignalKind])] = &[
        ("vitest", &[SignalKind::Test, SignalKind::Coverage]),
        ("@vitest/coverage-v8", &[SignalKind::Coverage]),
        ("eslint", &[SignalKind::Complexity]),
        ("@stylistic/eslint-plugin", &[SignalKind::Complexity]),
        ("@stryker-mutator/core", &[SignalKind::Mutation]),
    ];
    TOOLS
        .iter()
        .filter(|(_, signals)| request.requires_any(signals))
        .map(|(tool, signals)| {
            let direct = dependency(target_manifest, tool)?.map(|value| (target_root, value));
            let inherited = if owner_root == target_root {
                None
            } else {
                dependency(owner_manifest, tool)?.map(|value| (owner_root, value))
            };
            let declaration = direct.or(inherited);
            let (version, modifies_checkout, confidence, source_root, detail) =
                if let Some((root, value)) = declaration {
                    (
                        dependency_requirement(value)?,
                        false,
                        RequirementConfidence::Declared,
                        root,
                        Some(value),
                    )
                } else {
                    (
                        VersionRequirement::unresolved(
                            "project-integrated tool is not declared in package.json",
                        )
                        .map_err(adapter_error)?,
                        true,
                        RequirementConfidence::Assumed,
                        owner_root,
                        None,
                    )
                };
            Ok(SignalToolRequirement {
                tool: (*tool).to_string(),
                version,
                provider: String::from("node_project_dependency"),
                scope: ToolInstallationScope::Project,
                signals: signals.to_vec(),
                supported_platforms: request.requested_platforms().to_vec(),
                provisioning: ProvisioningSupport::OnlineOnly,
                modifies_checkout,
                source: source(
                    request.repo_root(),
                    &source_root.join("package.json"),
                    "package_json_dependency",
                    detail,
                    confidence,
                )?,
            })
        })
        .collect()
}

fn workspace_owner(repo_root: &Path, target_root: &Path) -> Result<PathBuf, AdapterError> {
    let mut current = target_root.parent();
    while let Some(root) = current.filter(|root| root.starts_with(repo_root)) {
        let Some(manifest) = read_manifest(repo_root, &root.join("package.json"), false)? else {
            if root == repo_root {
                break;
            }
            current = root.parent();
            continue;
        };
        let patterns = workspace_patterns(&manifest, &root.join("package.json"))?;
        if !patterns.is_empty() {
            let relative_target = target_root
                .strip_prefix(root)
                .map_err(|_| adapter_error("Node target escapes workspace root"))?
                .to_string_lossy()
                .replace('\\', "/");
            if workspace_matches(&patterns, &relative_target)? {
                return Ok(root.to_path_buf());
            }
        }
        if root == repo_root {
            break;
        }
        current = root.parent();
    }
    Ok(target_root.to_path_buf())
}

fn workspace_patterns(
    manifest: &serde_json::Value,
    path: &Path,
) -> Result<Vec<String>, AdapterError> {
    let Some(workspaces) = manifest.get("workspaces") else {
        return Ok(Vec::new());
    };
    let values = if let Some(array) = workspaces.as_array() {
        array
    } else if let Some(array) = workspaces
        .get("packages")
        .and_then(serde_json::Value::as_array)
    {
        array
    } else {
        return Err(adapter_error(format!(
            "{} workspaces must be an array or an object with a packages array",
            path.display()
        )));
    };
    values
        .iter()
        .map(|value| {
            value.as_str().map(str::to_owned).ok_or_else(|| {
                adapter_error(format!(
                    "{} workspaces must contain only string patterns",
                    path.display()
                ))
            })
        })
        .collect()
}

fn workspace_matches(patterns: &[String], target: &str) -> Result<bool, AdapterError> {
    let mut included = false;
    for raw in patterns {
        let (excluded, pattern) = raw
            .strip_prefix('!')
            .map_or((false, raw.as_str()), |value| (true, value));
        let pattern = Pattern::new(pattern).map_err(|error| {
            adapter_error(format!("invalid Node workspace pattern {raw}: {error}"))
        })?;
        if pattern.matches(target) {
            if excluded {
                return Ok(false);
            }
            included = true;
        }
    }
    Ok(included)
}

fn dependency_locks(
    repo_root: &Path,
    owner: &Path,
) -> Result<Vec<DependencyLockRequirement>, AdapterError> {
    lock_paths(repo_root, owner)?
        .into_iter()
        .map(|path| {
            let bytes = read_optional_contained_bytes(repo_root, &path)
                .map_err(adapter_error)?
                .expect("existing lock");
            if bytes.is_empty() {
                return Err(adapter_error(format!(
                    "native Node lockfile {} is empty",
                    path.display()
                )));
            }
            let relative_path = relative(repo_root, &path)?;
            Ok(DependencyLockRequirement {
                path: relative_path.clone(),
                digest: format!("sha256:{:x}", Sha256::digest(bytes)),
                owner_root: relative(repo_root, owner)?,
                source: source(
                    repo_root,
                    &path,
                    "node_dependency_lock",
                    lock_family(&relative_path),
                    RequirementConfidence::Exact,
                )?,
            })
        })
        .collect()
}

fn lock_paths(repo_root: &Path, owner: &Path) -> Result<Vec<PathBuf>, AdapterError> {
    let mut paths = Vec::new();
    for name in [
        "pnpm-lock.yaml",
        "yarn.lock",
        "package-lock.json",
        "bun.lock",
        "bun.lockb",
    ] {
        let path = owner.join(name);
        if read_optional_contained_bytes(repo_root, &path)
            .map_err(adapter_error)?
            .is_some()
        {
            paths.push(path);
        }
    }
    Ok(paths)
}

fn lock_family(path: &str) -> Option<&'static str> {
    if path.ends_with("pnpm-lock.yaml") {
        Some("pnpm")
    } else if path.ends_with("yarn.lock") {
        Some("yarn")
    } else if path.ends_with("package-lock.json") {
        Some("npm")
    } else if path.ends_with("bun.lock") || path.ends_with("bun.lockb") {
        Some("bun")
    } else {
        None
    }
}

fn parse_package_manager(value: &str) -> Result<(String, VersionRequirement), AdapterError> {
    let (family, version) = value
        .split_once('@')
        .ok_or_else(|| adapter_error("packageManager must use the form <family>@<version>"))?;
    let manager =
        PackageManager::from_executable(&family.to_ascii_lowercase()).ok_or_else(|| {
            adapter_error(format!("unsupported Node package manager family {family}"))
        })?;
    if version.trim().is_empty() {
        return Err(adapter_error("packageManager version must not be empty"));
    }
    let requirement = if is_exact_semver(version) {
        VersionRequirement::exact(version).map_err(adapter_error)?
    } else {
        VersionRequirement::selector(version).map_err(adapter_error)?
    };
    Ok((manager.executable().to_string(), requirement))
}

fn dependency<'a>(
    manifest: &'a serde_json::Value,
    name: &str,
) -> Result<Option<&'a str>, AdapterError> {
    for section in [
        "dependencies",
        "devDependencies",
        "optionalDependencies",
        "peerDependencies",
    ] {
        let Some(value) = manifest.get(section) else {
            continue;
        };
        let dependencies = value
            .as_object()
            .ok_or_else(|| adapter_error(format!("package.json {section} must be an object")))?;
        let Some(version) = dependencies.get(name) else {
            continue;
        };
        return version.as_str().map(Some).ok_or_else(|| {
            adapter_error(format!("package.json {section}.{name} must be a string"))
        });
    }
    Ok(None)
}

fn dependency_requirement(value: &str) -> Result<VersionRequirement, AdapterError> {
    if is_exact_semver(value) {
        VersionRequirement::exact(value).map_err(adapter_error)
    } else {
        VersionRequirement::compatibility(value).map_err(adapter_error)
    }
}

fn selector_requirement(value: &str) -> Result<VersionRequirement, AdapterError> {
    let normalized = value.strip_prefix('v').unwrap_or(value);
    if is_exact_semver(normalized) {
        VersionRequirement::exact(normalized).map_err(adapter_error)
    } else {
        VersionRequirement::selector(value).map_err(adapter_error)
    }
}

fn is_exact_semver(value: &str) -> bool {
    let value = value.strip_prefix('v').unwrap_or(value);
    let version = value.split('+').next().unwrap_or(value);
    let core = version.split('-').next().unwrap_or(version);
    let parts = core.split('.').collect::<Vec<_>>();
    parts.len() == 3
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

fn read_manifest(
    repo_root: &Path,
    path: &Path,
    required: bool,
) -> Result<Option<serde_json::Value>, AdapterError> {
    let content = if required {
        Some(read_contained_string(repo_root, path).map_err(adapter_error)?)
    } else {
        read_optional_contained_string(repo_root, path).map_err(adapter_error)?
    };
    content
        .map(|content| {
            serde_json::from_str(&content).map_err(|error| {
                adapter_error(format!("failed to parse {}: {error}", path.display()))
            })
        })
        .transpose()
}

fn relative(repo_root: &Path, path: &Path) -> Result<String, AdapterError> {
    repository_relative(repo_root, path).map_err(adapter_error)
}

fn source(
    repo_root: &Path,
    path: &Path,
    kind: &str,
    detail: Option<&str>,
    confidence: RequirementConfidence,
) -> Result<RequirementSource, AdapterError> {
    RequirementSource::new(kind, relative(repo_root, path)?, detail, confidence)
        .map_err(adapter_error)
}

fn adapter_error(error: impl std::fmt::Display) -> AdapterError {
    AdapterError::new(Language::Node, error.to_string())
}

#[cfg(test)]
mod tests;
