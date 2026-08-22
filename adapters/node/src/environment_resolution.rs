use ayni_adapters_common::repository::{read_optional_contained_bytes, repository_relative};
use ayni_core::{
    AdapterError, EnvironmentResolutionCapability, EnvironmentResolutionRequest, Language,
    RequirementConfidence, TargetEnvironment, VersionRequirement, sha256_fingerprint,
};
use node_semver::{Range, Version};
use std::collections::BTreeSet;
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
    let context = project_lock_context(request)?;
    let (content, mut source) = read_verified_project_lock(request, &context)?;
    let target_root = lockfile_target_root(context.owner, &request.target().target.root)?;
    let version = project_lock_tool_version(&context, &content, &target_root, tool)?;
    ensure_locked_version_matches(tool, &version, requirement, context.name)?;
    source.kind = format!("node_{}_lock_tool", context.family);
    source.detail = Some(tool.to_owned());
    source.confidence = RequirementConfidence::Exact;
    Ok((
        VersionRequirement::exact(version).map_err(|cause| error(cause.to_string()))?,
        source,
    ))
}

struct ProjectLockContext<'a> {
    family: &'a str,
    name: &'static str,
    owner: &'a str,
    path: std::path::PathBuf,
}

fn project_lock_context(
    request: &EnvironmentResolutionRequest,
) -> Result<ProjectLockContext<'_>, AdapterError> {
    let target = request.target();
    let owner = target
        .package_manager
        .as_ref()
        .map(|manager| manager.ownership_root.as_str())
        .unwrap_or(&target.target.root);
    let family = target
        .package_manager
        .as_ref()
        .map(|manager| manager.family.as_str())
        .unwrap_or("npm");
    let name = match family {
        "npm" => "package-lock.json",
        "pnpm" => "pnpm-lock.yaml",
        unsupported => {
            return Err(error(format!(
                "managed Node tool resolution does not support {unsupported}"
            )));
        }
    };
    let root = if owner == "." {
        request.repo_root().to_path_buf()
    } else {
        request.repo_root().join(owner)
    };
    Ok(ProjectLockContext {
        family,
        name,
        owner,
        path: root.join(name),
    })
}

fn read_verified_project_lock(
    request: &EnvironmentResolutionRequest,
    context: &ProjectLockContext<'_>,
) -> Result<(String, ayni_core::RequirementSource), AdapterError> {
    let relative = repository_relative(request.repo_root(), &context.path).map_err(error)?;
    let bytes = read_optional_contained_bytes(request.repo_root(), &context.path)
        .map_err(error)?
        .ok_or_else(|| error(format!("missing {}", context.path.display())))?;
    let digest = sha256_fingerprint(&bytes);
    let recorded = request
        .target()
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
        .map_err(|cause| error(format!("{} is not UTF-8: {cause}", context.path.display())))?;
    Ok((content, recorded.source.clone()))
}

fn project_lock_tool_version(
    context: &ProjectLockContext<'_>,
    content: &str,
    target_root: &str,
    tool: &str,
) -> Result<String, AdapterError> {
    let version = match context.family {
        "npm" => {
            let lock: serde_json::Value = serde_json::from_str(content).map_err(|cause| {
                error(format!(
                    "failed to parse {}: {cause}",
                    context.path.display()
                ))
            })?;
            locked_tool_version(&lock, target_root, tool).map(str::to_owned)
        }
        "pnpm" => pnpm_locked_tool_version(content, target_root, tool),
        _ => unreachable!("family validated by project_lock_context"),
    };
    version.ok_or_else(|| {
        error(format!(
            "{tool} is not resolved for {target_root} in {}; environment locking will not modify the checkout",
            context.name
        ))
    })
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

fn meaningful_yaml_line(line: &&str) -> bool {
    let value = line.trim();
    !value.is_empty() && !value.starts_with('#')
}

fn pnpm_locked_tool_version(content: &str, target_root: &str, tool: &str) -> Option<String> {
    pnpm_importer_tool_version(content, target_root, tool)
        .or_else(|| {
            (target_root != ".")
                .then(|| pnpm_importer_tool_version(content, ".", tool))
                .flatten()
        })
        .or_else(|| pnpm_package_version(content, tool))
}

fn pnpm_importer_tool_version(content: &str, target_root: &str, tool: &str) -> Option<String> {
    let mut state = PnpmImporterState::default();
    for line in content.lines().filter(meaningful_yaml_line) {
        let trimmed = line.trim();
        let indent = line.len().saturating_sub(line.trim_start().len());
        let version = match indent {
            0 => {
                state.enter_root(trimmed);
                None
            }
            2 if state.in_importers => state.enter_importer(trimmed, target_root),
            4 if state.active_importer => state.enter_dependency_section(trimmed),
            6 if state.in_dependencies => state.enter_tool(trimmed, tool),
            8 if state.active_tool => state.read_tool_version(trimmed),
            _ => None,
        };
        if version.is_some() {
            return version;
        }
    }
    None
}

#[derive(Default)]
struct PnpmImporterState {
    in_importers: bool,
    active_importer: bool,
    in_dependencies: bool,
    active_tool: bool,
}

impl PnpmImporterState {
    fn enter_root(&mut self, value: &str) {
        self.in_importers = value == "importers:";
        self.active_importer = false;
        self.in_dependencies = false;
        self.active_tool = false;
    }

    fn enter_importer(&mut self, value: &str, target_root: &str) -> Option<String> {
        let (key, _) = yaml_mapping(value)?;
        self.active_importer = key == target_root;
        self.in_dependencies = false;
        self.active_tool = false;
        None
    }

    fn enter_dependency_section(&mut self, value: &str) -> Option<String> {
        let (key, _) = yaml_mapping(value)?;
        self.in_dependencies = matches!(
            key,
            "dependencies" | "devDependencies" | "optionalDependencies"
        );
        self.active_tool = false;
        None
    }

    fn enter_tool(&mut self, value: &str, tool: &str) -> Option<String> {
        let (key, version) = yaml_mapping(value)?;
        self.active_tool = key == tool;
        (self.active_tool && !version.is_empty())
            .then(|| normalize_pnpm_version(version))
            .flatten()
    }

    fn read_tool_version(&self, value: &str) -> Option<String> {
        let (key, version) = yaml_mapping(value)?;
        (key == "version")
            .then(|| normalize_pnpm_version(version))
            .flatten()
    }
}

fn pnpm_package_version(content: &str, tool: &str) -> Option<String> {
    let mut in_packages = false;
    let mut versions = BTreeSet::new();
    for line in content.lines().filter(meaningful_yaml_line) {
        let trimmed = line.trim();
        let indent = line.len().saturating_sub(line.trim_start().len());
        if indent == 0 {
            in_packages = trimmed == "packages:";
            continue;
        }
        if !in_packages || indent != 2 {
            continue;
        }
        let Some((coordinate, _)) = yaml_mapping(trimmed) else {
            continue;
        };
        let Some(version) = coordinate
            .strip_prefix(tool)
            .and_then(|value| value.strip_prefix('@'))
        else {
            continue;
        };
        if let Some(version) = normalize_pnpm_version(version) {
            versions.insert(version);
        }
    }
    (versions.len() == 1)
        .then(|| versions.into_iter().next())
        .flatten()
}

fn yaml_mapping(value: &str) -> Option<(&str, &str)> {
    if let Some(quote) = value
        .chars()
        .next()
        .filter(|quote| matches!(quote, '\'' | '"'))
    {
        let rest = &value[quote.len_utf8()..];
        let end = rest.find(quote)?;
        let key = &rest[..end];
        let value = rest[end + quote.len_utf8()..].strip_prefix(':')?.trim();
        Some((key, value))
    } else {
        let (key, value) = value.split_once(':')?;
        Some((key.trim(), value.trim()))
    }
}

fn normalize_pnpm_version(value: &str) -> Option<String> {
    let value = value.trim().trim_matches(['\'', '"']);
    let version = value.split('(').next()?.trim();
    (!version.is_empty()).then(|| version.to_owned())
}

fn ensure_locked_version_matches(
    tool: &str,
    version: &str,
    requirement: &VersionRequirement,
    lock_name: &str,
) -> Result<(), AdapterError> {
    if matches!(requirement, VersionRequirement::Unresolved { .. }) {
        Version::parse(version)
            .map_err(|cause| error(format!("invalid locked {tool} version {version}: {cause}")))?;
        return Ok(());
    }
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
            "{lock_name} resolves {tool} to {version}, which does not satisfy {expression}"
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
        assert!(
            ensure_locked_version_matches("eslint", "8.57.1", &requirement, "package-lock.json")
                .is_ok()
        );
        assert!(
            ensure_locked_version_matches("eslint", "9.0.0", &requirement, "package-lock.json")
                .is_err()
        );
        let transitive = VersionRequirement::unresolved("not directly declared").unwrap();
        assert!(
            ensure_locked_version_matches(
                "@typescript-eslint/parser",
                "8.49.0",
                &transitive,
                "pnpm-lock.yaml",
            )
            .is_ok()
        );
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
    fn resolves_pnpm_importer_tool_versions_with_peer_suffixes() {
        let lock = r#"
lockfileVersion: '9.0'
importers:
  .:
    devDependencies:
      vitest:
        specifier: ^3.2.0
        version: 3.2.4(@types/node@24.0.0)
  apps/web:
    devDependencies:
      '@vitest/coverage-v8':
        specifier: 3.2.4
        version: 3.2.4(vitest@3.2.4)
packages:
  '@typescript-eslint/parser@8.49.0': {}
"#;
        assert_eq!(
            pnpm_locked_tool_version(lock, ".", "vitest"),
            Some(String::from("3.2.4"))
        );
        assert_eq!(
            pnpm_locked_tool_version(lock, "apps/web", "@vitest/coverage-v8"),
            Some(String::from("3.2.4"))
        );
        assert_eq!(
            pnpm_locked_tool_version(lock, ".", "@typescript-eslint/parser"),
            Some(String::from("8.49.0"))
        );
    }

    #[test]
    fn pnpm_fallback_requires_one_unambiguous_locked_version() {
        let lock = r#"
lockfileVersion: '9.0'
importers:
  .:
    devDependencies:
      vitest:
        version: 3.2.4
  apps/web: {}
packages:
  '@typescript-eslint/parser@8.48.0': {}
  '@typescript-eslint/parser@8.49.0': {}
"#;
        assert_eq!(
            pnpm_locked_tool_version(lock, "apps/web", "vitest"),
            Some(String::from("3.2.4"))
        );
        assert_eq!(
            pnpm_locked_tool_version(lock, "apps/web", "@typescript-eslint/parser"),
            None
        );
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
