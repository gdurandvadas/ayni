use ayni_adapters_common::repository::{read_optional_contained_bytes, repository_relative};
use ayni_core::{
    AdapterError, EnvironmentResolutionCapability, EnvironmentResolutionRequest, Language,
    RequirementConfidence, TargetEnvironment, VersionRequirement,
};
use node_semver::{Range, Version};
use sha2::{Digest, Sha256};
use std::time::Duration;

#[derive(Debug, Default)]
pub(crate) struct NodeEnvironmentResolutionCapability;

impl EnvironmentResolutionCapability for NodeEnvironmentResolutionCapability {
    fn language(&self) -> Language {
        Language::Node
    }

    fn resolve(
        &self,
        request: &EnvironmentResolutionRequest,
    ) -> Result<TargetEnvironment, AdapterError> {
        let mut target = request.target().clone();
        for runtime in &mut target.runtimes {
            runtime.version = resolve_registry(request, "node", &runtime.version)?;
        }
        if let Some(manager) = &mut target.package_manager {
            manager.version = resolve_registry(request, &manager.family, &manager.version)?;
        }
        for tool in &mut target.signal_tools {
            let (version, source) = resolve_project_tool(request, &tool.tool, &tool.version)?;
            tool.version = version;
            tool.source = source;
            tool.modifies_checkout = false;
        }
        Ok(target)
    }
}

fn resolve_registry(
    request: &EnvironmentResolutionRequest,
    name: &str,
    requirement: &VersionRequirement,
) -> Result<VersionRequirement, AdapterError> {
    if let VersionRequirement::Exact { .. } = requirement {
        return Ok(requirement.clone());
    }
    let expression = requirement_expression(name, requirement)?;
    let range = expression.parse::<Range>().map_err(|cause| {
        error(format!(
            "invalid {name} version selector {expression}: {cause}"
        ))
    })?;
    let args = vec![
        "--no-config".to_owned(),
        "--no-env".to_owned(),
        "--no-hooks".to_owned(),
        "ls-remote".to_owned(),
        name.to_owned(),
    ];
    let output = ayni_adapters_common::exec::run_command(
        request.repo_root(),
        "mise",
        &args,
        Duration::from_secs(120),
    )
    .map_err(|cause| provider_error(format!("failed to run mise for {name}: {cause}")))?;
    if !output.status.success() {
        return Err(provider_error(format!(
            "mise could not list versions for {name}"
        )));
    }
    let stdout = String::from_utf8(output.stdout).map_err(|cause| {
        provider_error(format!(
            "mise returned non-UTF-8 output for {name}: {cause}"
        ))
    })?;
    let version = select_version(&stdout, &range).ok_or_else(|| {
        error(format!(
            "mise returned no exact version matching {name}@{expression}"
        ))
    })?;
    VersionRequirement::exact(version.to_string()).map_err(|cause| error(cause.to_string()))
}

fn select_version(output: &str, range: &Range) -> Option<Version> {
    output
        .lines()
        .filter_map(|line| Version::parse(line.trim().trim_start_matches('v')).ok())
        .filter(|candidate| candidate.satisfies(range))
        .max()
}

fn requirement_expression(
    name: &str,
    requirement: &VersionRequirement,
) -> Result<String, AdapterError> {
    match requirement {
        VersionRequirement::Selector { expression }
        | VersionRequirement::Compatibility { expression } => Ok(expression.clone()),
        VersionRequirement::Minimum { version } => Ok(format!(">={version}")),
        VersionRequirement::Unresolved { reason } => {
            Err(error(format!("cannot resolve {name}: {reason}")))
        }
        VersionRequirement::Exact { version } => Ok(version.clone()),
    }
}

fn resolve_project_tool(
    request: &EnvironmentResolutionRequest,
    tool: &str,
    requirement: &VersionRequirement,
) -> Result<(VersionRequirement, ayni_core::RequirementSource), AdapterError> {
    let target = request.target();
    let owner = target
        .package_manager
        .as_ref()
        .map(|manager| manager.ownership_root.as_str())
        .unwrap_or(&target.target.root);
    let root = if owner == "." {
        request.repo_root().to_path_buf()
    } else {
        request.repo_root().join(owner)
    };
    let path = root.join("package-lock.json");
    let relative = repository_relative(request.repo_root(), &path).map_err(error)?;
    let bytes = read_optional_contained_bytes(request.repo_root(), &path)
        .map_err(error)?
        .ok_or_else(|| error(format!("missing {}", path.display())))?;
    let digest = format!("sha256:{:x}", Sha256::digest(&bytes));
    let recorded = target
        .dependency_locks
        .iter()
        .find(|dependency| dependency.path == relative)
        .ok_or_else(|| {
            error(format!(
                "{relative} was not recorded during environment discovery"
            ))
        })?;
    if recorded.digest != digest {
        return Err(error(format!(
            "{relative} changed during environment locking; rerun the command"
        )));
    }
    let content = String::from_utf8(bytes)
        .map_err(|cause| error(format!("{} is not UTF-8: {cause}", path.display())))?;
    let lock: serde_json::Value = serde_json::from_str(&content)
        .map_err(|cause| error(format!("failed to parse {}: {cause}", path.display())))?;
    let lock_target_root = lockfile_target_root(owner, &target.target.root)?;
    let version = locked_tool_version(&lock, &lock_target_root, tool).ok_or_else(|| {
        error(format!(
            "{tool} is not resolved for {} in package-lock.json; environment locking will not modify the checkout",
            target.target.root
        ))
    })?;
    ensure_locked_version_matches(tool, version, requirement)?;
    let version = VersionRequirement::exact(version).map_err(|cause| error(cause.to_string()))?;
    let mut source = recorded.source.clone();
    source.kind = "node_package_lock_tool".to_owned();
    source.detail = Some(tool.to_owned());
    source.confidence = RequirementConfidence::Exact;
    Ok((version, source))
}

fn lockfile_target_root(owner: &str, target: &str) -> Result<String, AdapterError> {
    if owner == "." {
        return Ok(target.to_owned());
    }
    let relative = std::path::Path::new(target)
        .strip_prefix(owner)
        .map_err(|_| {
            error(format!(
                "target {target} is outside package-lock owner {owner}"
            ))
        })?;
    let value = relative.to_string_lossy().replace('\\', "/");
    Ok(if value.is_empty() {
        ".".to_owned()
    } else {
        value
    })
}

fn locked_tool_version<'a>(
    lock: &'a serde_json::Value,
    target_root: &str,
    tool: &str,
) -> Option<&'a str> {
    let local_key = if target_root == "." {
        format!("node_modules/{tool}")
    } else {
        format!("{target_root}/node_modules/{tool}")
    };
    let hoisted_key = format!("node_modules/{tool}");
    [local_key.as_str(), hoisted_key.as_str()]
        .into_iter()
        .find_map(|key| {
            lock.get("packages")
                .and_then(|packages| packages.get(key))
                .and_then(|package| package.get("version"))
                .and_then(serde_json::Value::as_str)
        })
        .or_else(|| {
            lock.get("dependencies")
                .and_then(|dependencies| dependencies.get(tool))
                .and_then(|dependency| dependency.get("version"))
                .and_then(serde_json::Value::as_str)
        })
}

fn ensure_locked_version_matches(
    tool: &str,
    version: &str,
    requirement: &VersionRequirement,
) -> Result<(), AdapterError> {
    let expression = requirement_expression(tool, requirement)?;
    let range = expression
        .parse::<Range>()
        .map_err(|cause| error(format!("invalid {tool} requirement {expression}: {cause}")))?;
    let version = Version::parse(version)
        .map_err(|cause| error(format!("invalid locked {tool} version {version}: {cause}")))?;
    if version.satisfies(&range) {
        Ok(())
    } else {
        Err(error(format!(
            "package-lock.json resolves {tool} to {version}, which does not satisfy {expression}"
        )))
    }
}

fn provider_error(message: impl Into<String>) -> AdapterError {
    AdapterError::execution(Language::Node, message)
}

fn error(message: impl std::fmt::Display) -> AdapterError {
    AdapterError::new(Language::Node, message.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_highest_version_satisfying_a_bounded_range() {
        let range = ">=20 <23".parse::<Range>().expect("valid range");
        let selected = select_version("v18.20.0\n20.15.1\n22.12.0\n23.1.0\n", &range);
        assert_eq!(selected.expect("matching version").to_string(), "22.12.0");
    }

    #[test]
    fn validates_package_lock_version_against_manifest_requirement() {
        let requirement = VersionRequirement::compatibility("^8.0.0").expect("requirement");
        assert!(ensure_locked_version_matches("eslint", "8.57.1", &requirement).is_ok());
        assert!(ensure_locked_version_matches("eslint", "9.0.0", &requirement).is_err());
    }

    #[test]
    fn target_root_is_relative_to_nested_package_lock_owner() {
        assert_eq!(
            lockfile_target_root("frontend", "frontend/apps/web").expect("relative target"),
            "apps/web"
        );
        assert!(lockfile_target_root("frontend", "backend/api").is_err());
    }

    #[test]
    fn prefers_workspace_local_package_lock_resolution_over_hoisted_version() {
        let lock = serde_json::json!({
            "packages": {
                "node_modules/vitest": { "version": "3.1.0" },
                "apps/web/node_modules/vitest": { "version": "3.2.4" }
            }
        });
        assert_eq!(
            locked_tool_version(&lock, "apps/web", "vitest"),
            Some("3.2.4")
        );
    }
}
