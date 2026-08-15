//! Read-only Go environment discovery.

use crate::catalog::{GOCYCLO_MODULE, GOCYCLO_VERSION};
use ayni_adapters_common::repository::{
    read_contained_string, read_optional_contained_bytes, read_optional_contained_string,
    repository_relative,
};
use ayni_core::{
    AdapterError, DependencyLockRequirement, EnvironmentCapability, EnvironmentConflict,
    EnvironmentContribution, EnvironmentDiscoveryRequest, EnvironmentWarning, Language,
    ProvisioningSupport, RequirementConfidence, RequirementSource, RuntimeRequirement, SignalKind,
    SignalToolRequirement, TargetEnvironment, ToolInstallationScope, VersionRequirement,
};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Default)]
pub(crate) struct GoEnvironmentCapability;

impl EnvironmentCapability for GoEnvironmentCapability {
    fn language(&self) -> Language {
        Language::Go
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
    let module = parse_module(request.repo_root(), &target_root.join("go.mod"))?;
    let workspace = find_workspace(request.repo_root(), &target_root)?;
    let workspace_modules = workspace
        .as_ref()
        .map(|workspace| workspace.modules.as_slice())
        .unwrap_or(&[]);
    let workspace_contains_target = workspace_modules.iter().any(|root| root == &target_root);
    let governing_workspace = workspace.as_ref().filter(|_| workspace_contains_target);
    let (runtime, mut conflicts, mut warnings) =
        runtime_requirement(request, &target_root, &module, governing_workspace)?;
    if workspace.is_some() && !workspace_contains_target {
        warnings.push(EnvironmentWarning {
            code: String::from("go_workspace_target_unlisted"),
            message: format!(
                "{} is not listed by the containing go.work; it is owned as an independent module.",
                target_root.display()
            ),
            target: Some(request.target().clone()),
        });
    }
    let owner_root = workspace
        .as_ref()
        .filter(|_| workspace_contains_target)
        .map_or_else(|| target_root.clone(), |workspace| workspace.root.clone());

    let dependency_locks = dependency_locks(
        request.repo_root(),
        &owner_root,
        &target_root,
        workspace.as_ref(),
    )?;
    let target_sum = relative(request.repo_root(), &target_root.join("go.sum"))?;
    if module.has_requirements && !dependency_locks.iter().any(|lock| lock.path == target_sum) {
        conflicts.push(EnvironmentConflict {
            code: String::from("go_checksum_lock_missing"),
            message: format!(
                "{} is missing; Go dependency preparation requires a committed go.sum.",
                target_root.join("go.sum").display()
            ),
            target: Some(request.target().clone()),
            sources: vec![source(
                request.repo_root(),
                "go_module_input",
                &target_root.join("go.mod"),
                Some("requires dependencies"),
                RequirementConfidence::Declared,
            )?],
        });
    }

    EnvironmentContribution::new(
        TargetEnvironment {
            target: request.target().clone(),
            workspace: (owner_root != target_root)
                .then(|| relative(request.repo_root(), &owner_root))
                .transpose()?,
            package: Some(module.name),
            runtimes: vec![runtime],
            package_manager: None,
            signal_tools: signal_tools(request, &target_root.join("go.mod"))?,
            system_requirements: Vec::new(),
            dependency_locks,
        },
        warnings,
        conflicts,
    )
    .map_err(plan_error)
}

#[derive(Debug)]
struct Module {
    name: String,
    go: Option<Directive>,
    toolchain: Option<Directive>,
    has_requirements: bool,
    local_replacements: Vec<PathBuf>,
}

#[derive(Debug)]
struct Workspace {
    root: PathBuf,
    go: Option<Directive>,
    toolchain: Option<Directive>,
    modules: Vec<PathBuf>,
    local_replacements: Vec<PathBuf>,
}

#[derive(Debug, Clone)]
struct Directive {
    value: String,
    source: RequirementSource,
}

fn parse_module(repo_root: &Path, path: &Path) -> Result<Module, AdapterError> {
    let content = read_contained_string(repo_root, path).map_err(error)?;
    let module = directive(repo_root, &content, "module", path)?
        .ok_or_else(|| error(format!("{} must declare a module", path.display())))?;
    let name = module.value;
    let has_requirements = content.lines().any(|line| {
        let line = line
            .split_once("//")
            .map_or(line, |(before, _)| before)
            .trim();
        line.starts_with("require ") || line == "require("
    });
    Ok(Module {
        name,
        go: directive(repo_root, &content, "go", path)?,
        toolchain: directive(repo_root, &content, "toolchain", path)?,
        has_requirements,
        local_replacements: local_replacements(repo_root, path, &content)?,
    })
}

fn find_workspace(repo_root: &Path, target_root: &Path) -> Result<Option<Workspace>, AdapterError> {
    let mut current = Some(target_root);
    while let Some(root) = current {
        let path = root.join("go.work");
        if read_optional_contained_string(repo_root, &path)
            .map_err(error)?
            .is_some()
        {
            return parse_workspace(repo_root, &path).map(Some);
        }
        if root == repo_root {
            break;
        }
        current = root.parent().filter(|parent| parent.starts_with(repo_root));
    }
    Ok(None)
}

fn parse_workspace(repo_root: &Path, path: &Path) -> Result<Workspace, AdapterError> {
    let content = read_contained_string(repo_root, path).map_err(error)?;
    let root = path
        .parent()
        .ok_or_else(|| error("go.work has no parent directory"))?
        .to_path_buf();
    let mut modules = workspace_uses(&content, path)?
        .into_iter()
        .map(|value| resolve_workspace_module(repo_root, &root, &value))
        .collect::<Result<Vec<_>, _>>()?;
    modules.sort();
    modules.dedup();
    if modules.is_empty() {
        return Err(error(format!(
            "{} must declare at least one use path",
            path.display()
        )));
    }
    Ok(Workspace {
        root,
        go: directive(repo_root, &content, "go", path)?,
        toolchain: directive(repo_root, &content, "toolchain", path)?,
        modules,
        local_replacements: local_replacements(repo_root, path, &content)?,
    })
}

fn workspace_uses(content: &str, path: &Path) -> Result<Vec<String>, AdapterError> {
    let mut uses = Vec::new();
    let mut block = false;
    for raw in content.lines() {
        let line = raw
            .split_once("//")
            .map_or(raw, |(before, _)| before)
            .trim();
        if line.is_empty() {
            continue;
        }
        if block {
            if line == ")" {
                block = false;
                continue;
            }
            if line.contains(char::is_whitespace) {
                return Err(error(format!("{} has malformed use entry", path.display())));
            }
            uses.push(unquote_path(line, path)?);
            continue;
        }
        if line == "use (" {
            block = true;
            continue;
        }
        if let Some(value) = line.strip_prefix("use ") {
            if value.is_empty() || value.contains(char::is_whitespace) {
                return Err(error(format!(
                    "{} has malformed use directive",
                    path.display()
                )));
            }
            uses.push(unquote_path(value, path)?);
        }
    }
    if block {
        return Err(error(format!(
            "{} has an unterminated use block",
            path.display()
        )));
    }
    Ok(uses)
}

fn unquote_path(value: &str, path: &Path) -> Result<String, AdapterError> {
    if value.starts_with('"') {
        serde_json::from_str(value).map_err(|cause| {
            error(format!(
                "{} has invalid quoted use path: {cause}",
                path.display()
            ))
        })
    } else {
        Ok(value.to_owned())
    }
}

fn resolve_workspace_module(
    repo_root: &Path,
    workspace_root: &Path,
    value: &str,
) -> Result<PathBuf, AdapterError> {
    let candidate = workspace_root.join(value);
    let canonical = candidate.canonicalize().map_err(|cause| {
        error(format!(
            "failed to resolve go.work use path {}: {cause}",
            candidate.display()
        ))
    })?;
    if !canonical.starts_with(repo_root)
        || !canonical.is_dir()
        || !canonical.join("go.mod").is_file()
    {
        return Err(error(format!(
            "go.work use path is not a repository-contained module: {}",
            candidate.display()
        )));
    }
    Ok(canonical)
}

fn local_replacements(
    repo_root: &Path,
    manifest: &Path,
    content: &str,
) -> Result<Vec<PathBuf>, AdapterError> {
    let mut replacements = Vec::new();
    let mut block = false;
    for raw in content.lines() {
        let line = raw
            .split_once("//")
            .map_or(raw, |(before, _)| before)
            .trim();
        let Some(value) = replacement_directive(line, manifest, &mut block)? else {
            continue;
        };
        if let Some(path) = local_replacement_path(repo_root, manifest, value)? {
            replacements.push(path);
        }
    }
    if block {
        return Err(error(format!(
            "{} has an unterminated replace block",
            manifest.display()
        )));
    }
    replacements.sort();
    replacements.dedup();
    Ok(replacements)
}

fn replacement_directive<'a>(
    line: &'a str,
    manifest: &Path,
    block: &mut bool,
) -> Result<Option<&'a str>, AdapterError> {
    if line.is_empty() {
        return Ok(None);
    }
    if *block && line == ")" {
        *block = false;
        return Ok(None);
    }
    if !*block && line == "replace (" {
        *block = true;
        return Ok(None);
    }
    let value = if *block {
        line
    } else if let Some(value) = line.strip_prefix("replace ") {
        value
    } else {
        return Ok(None);
    };
    value
        .split_once("=>")
        .map(|(_, value)| Some(value))
        .ok_or_else(|| {
            error(format!(
                "{} has a malformed replace directive",
                manifest.display()
            ))
        })
}

fn local_replacement_path(
    repo_root: &Path,
    manifest: &Path,
    replacement: &str,
) -> Result<Option<PathBuf>, AdapterError> {
    let parts = replacement.split_whitespace().collect::<Vec<_>>();
    if parts.len() != 1 {
        return Ok(None);
    }
    let value = parts[0];
    if !(value.starts_with('.') || Path::new(value).is_absolute()) {
        return Ok(None);
    }
    if Path::new(value).is_absolute() {
        return Err(error(format!(
            "{} uses a host-absolute local replacement {value}",
            manifest.display()
        )));
    }
    let candidate = manifest.parent().unwrap_or(repo_root).join(value);
    let canonical = candidate.canonicalize().map_err(|cause| {
        error(format!(
            "failed to resolve local Go replacement {}: {cause}",
            candidate.display()
        ))
    })?;
    if canonical.starts_with(repo_root) && canonical.is_dir() && canonical.join("go.mod").is_file()
    {
        Ok(Some(canonical))
    } else {
        Err(error(format!(
            "local Go replacement is not a repository-contained module: {}",
            candidate.display()
        )))
    }
}

fn directive(
    repo_root: &Path,
    content: &str,
    name: &str,
    path: &Path,
) -> Result<Option<Directive>, AdapterError> {
    let mut values = Vec::new();
    for raw in content.lines() {
        let line = raw
            .split_once("//")
            .map_or(raw, |(before, _)| before)
            .trim();
        let mut words = line.split_whitespace();
        if words.next() != Some(name) {
            continue;
        }
        let Some(value) = words.next() else {
            return Err(error(format!(
                "{} {name} directive needs one value",
                path.display()
            )));
        };
        if words.next().is_some() {
            return Err(error(format!(
                "{} {name} directive needs one value",
                path.display()
            )));
        }
        values.push(value.to_owned());
    }
    if values.len() > 1 {
        return Err(error(format!(
            "{} has multiple {name} directives",
            path.display()
        )));
    }
    values
        .pop()
        .map(|value| {
            Ok(Directive {
                value,
                source: source(
                    repo_root,
                    "go_directive",
                    path,
                    Some(name),
                    RequirementConfidence::Declared,
                )?,
            })
        })
        .transpose()
}

type GoRuntimeSelection = (
    VersionRequirement,
    RequirementSource,
    Vec<EnvironmentConflict>,
);

fn runtime_requirement(
    request: &EnvironmentDiscoveryRequest,
    target_root: &Path,
    module: &Module,
    workspace: Option<&Workspace>,
) -> Result<
    (
        RuntimeRequirement,
        Vec<EnvironmentConflict>,
        Vec<EnvironmentWarning>,
    ),
    AdapterError,
> {
    let selectors = runtime_selectors(request.repo_root(), target_root, module, workspace)?;
    let minimums = [
        workspace.and_then(|value| value.go.as_ref()),
        module.go.as_ref(),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();
    let minimum = maximum_go_directive(&minimums)?;
    let (version, source, conflicts) =
        select_go_runtime(request, target_root, &selectors, minimum.as_ref())?;
    let warnings = matches!(version, VersionRequirement::Unresolved { .. })
        .then(|| EnvironmentWarning {
            code: String::from("go_runtime_unresolved"),
            message: String::from(
                "No Go toolchain selector, toolchain directive, or go directive was found.",
            ),
            target: Some(request.target().clone()),
        })
        .into_iter()
        .collect();
    Ok((
        RuntimeRequirement {
            runtime: String::from("go"),
            version,
            components: Vec::new(),
            targets: Vec::new(),
            source,
        },
        conflicts,
        warnings,
    ))
}

fn runtime_selectors(
    repo_root: &Path,
    target_root: &Path,
    module: &Module,
    workspace: Option<&Workspace>,
) -> Result<Vec<Directive>, AdapterError> {
    let target = selector_files(repo_root, target_root)?;
    if !target.is_empty() {
        return Ok(target);
    }
    if let Some(workspace) = workspace {
        let workspace_selectors = selector_files(repo_root, &workspace.root)?;
        if !workspace_selectors.is_empty() {
            return Ok(workspace_selectors);
        }
    }
    Ok(workspace
        .and_then(|value| non_default_toolchain(value.toolchain.as_ref()))
        .or_else(|| non_default_toolchain(module.toolchain.as_ref()))
        .cloned()
        .into_iter()
        .collect())
}

fn non_default_toolchain(toolchain: Option<&Directive>) -> Option<&Directive> {
    toolchain.filter(|value| value.value != "default")
}

fn select_go_runtime(
    request: &EnvironmentDiscoveryRequest,
    target_root: &Path,
    selectors: &[Directive],
    minimum: Option<&Directive>,
) -> Result<GoRuntimeSelection, AdapterError> {
    let selector_values = selectors
        .iter()
        .map(|item| normalized_go_version(&item.value))
        .collect::<Result<BTreeSet<_>, _>>()?;
    if selector_values.len() > 1 {
        return conflicting_go_selectors(request, selectors, &selector_values);
    }
    if let Some(selector) = selectors.first() {
        return selected_go_toolchain(request, selector, minimum);
    }
    if let Some(minimum) = minimum {
        return Ok((
            VersionRequirement::minimum(&minimum.value).map_err(plan_error)?,
            minimum.source.clone(),
            Vec::new(),
        ));
    }
    Ok((
        VersionRequirement::unresolved("no Go toolchain selector or go directive")
            .map_err(plan_error)?,
        source(
            request.repo_root(),
            "go_runtime_unresolved",
            &target_root.join("go.mod"),
            None,
            RequirementConfidence::Assumed,
        )?,
        Vec::new(),
    ))
}

fn conflicting_go_selectors(
    request: &EnvironmentDiscoveryRequest,
    selectors: &[Directive],
    values: &BTreeSet<String>,
) -> Result<GoRuntimeSelection, AdapterError> {
    Ok((
        VersionRequirement::unresolved("conflicting Go toolchain selectors").map_err(plan_error)?,
        selectors[0].source.clone(),
        vec![EnvironmentConflict {
            code: String::from("go_runtime_requirement_conflict"),
            message: format!(
                "Go toolchain selectors disagree: {}",
                values.iter().cloned().collect::<Vec<_>>().join(", ")
            ),
            target: Some(request.target().clone()),
            sources: selectors.iter().map(|item| item.source.clone()).collect(),
        }],
    ))
}

fn selected_go_toolchain(
    request: &EnvironmentDiscoveryRequest,
    selector: &Directive,
    minimum: Option<&Directive>,
) -> Result<GoRuntimeSelection, AdapterError> {
    let normalized = normalized_go_version(&selector.value)?;
    let conflicts = minimum
        .filter(|minimum| {
            compare_go_versions(&normalized, &minimum.value)
                .is_ok_and(|ordering| ordering == std::cmp::Ordering::Less)
        })
        .map(|minimum| EnvironmentConflict {
            code: String::from("go_toolchain_below_minimum"),
            message: format!(
                "selected Go toolchain {normalized} is below the declared minimum {}",
                minimum.value
            ),
            target: Some(request.target().clone()),
            sources: vec![selector.source.clone(), minimum.source.clone()],
        })
        .into_iter()
        .collect();
    Ok((
        version_requirement(&normalized, false)?,
        selector.source.clone(),
        conflicts,
    ))
}

fn maximum_go_directive(values: &[&Directive]) -> Result<Option<Directive>, AdapterError> {
    let mut selected: Option<Directive> = None;
    for value in values {
        let normalized = normalized_go_version(&value.value)?;
        let candidate = Directive {
            value: normalized,
            source: value.source.clone(),
        };
        if selected.as_ref().is_none_or(|current| {
            compare_go_versions(&candidate.value, &current.value)
                .is_ok_and(|ordering| ordering == std::cmp::Ordering::Greater)
        }) {
            selected = Some(candidate);
        }
    }
    Ok(selected)
}

fn normalized_go_version(value: &str) -> Result<String, AdapterError> {
    let value = value.strip_prefix("go").unwrap_or(value);
    let valid = matches!(value.split('.').count(), 2 | 3)
        && value
            .split('.')
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()));
    if valid {
        Ok(value.to_owned())
    } else {
        Err(error(format!("invalid Go version requirement {value}")))
    }
}

fn compare_go_versions(left: &str, right: &str) -> Result<std::cmp::Ordering, AdapterError> {
    fn components(value: &str) -> Result<[u64; 3], AdapterError> {
        let mut output = [0_u64; 3];
        for (index, part) in value.split('.').enumerate() {
            output[index] = part
                .parse()
                .map_err(|_| error(format!("invalid Go version requirement {value}")))?;
        }
        Ok(output)
    }
    Ok(components(left)?.cmp(&components(right)?))
}

fn selector_files(repo_root: &Path, root: &Path) -> Result<Vec<Directive>, AdapterError> {
    let mut found = Vec::new();
    for name in [".go-version", ".tool-versions"] {
        let path = root.join(name);
        let Some(content) = read_optional_contained_string(repo_root, &path).map_err(error)? else {
            continue;
        };
        let value = if name == ".tool-versions" {
            content
                .lines()
                .find_map(|line| {
                    line.split_whitespace()
                        .collect::<Vec<_>>()
                        .as_slice()
                        .split_first()
                        .filter(|(tool, _)| **tool == "golang")
                        .and_then(|(_, versions)| versions.first().copied())
                })
                .map(str::to_owned)
        } else {
            Some(content.trim().to_owned())
        };
        let Some(value) = value else {
            continue;
        };
        if value.is_empty() || value.lines().count() != 1 || value.contains(char::is_whitespace) {
            return Err(error(format!(
                "{} must contain one non-empty Go selector",
                path.display()
            )));
        }
        found.push(Directive {
            value,
            source: source(
                repo_root,
                "go_runtime_selector",
                &path,
                None,
                RequirementConfidence::Declared,
            )?,
        });
    }
    Ok(found)
}

fn version_requirement(
    value: &str,
    go_directive: bool,
) -> Result<VersionRequirement, AdapterError> {
    let value = value.strip_prefix("go").unwrap_or(value);
    let exact = value.split('.').count() == 3
        && value
            .split('.')
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()));
    if exact {
        VersionRequirement::exact(value).map_err(plan_error)
    } else if go_directive
        && value.split('.').count() >= 2
        && value
            .split('.')
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
    {
        VersionRequirement::minimum(value).map_err(plan_error)
    } else {
        VersionRequirement::selector(value).map_err(plan_error)
    }
}

fn signal_tools(
    request: &EnvironmentDiscoveryRequest,
    manifest: &Path,
) -> Result<Vec<SignalToolRequirement>, AdapterError> {
    if !request.requires_any(&[SignalKind::Complexity]) {
        return Ok(Vec::new());
    }
    Ok(vec![SignalToolRequirement {
        tool: String::from("gocyclo"),
        version: VersionRequirement::exact(GOCYCLO_VERSION).map_err(plan_error)?,
        provider: format!("go:{GOCYCLO_MODULE}"),
        scope: ToolInstallationScope::Isolated,
        signals: vec![SignalKind::Complexity],
        supported_platforms: request.requested_platforms().to_vec(),
        provisioning: ProvisioningSupport::OnlineOnly,
        modifies_checkout: false,
        source: source(
            request.repo_root(),
            "go_adapter_catalog",
            manifest,
            Some("gocyclo"),
            RequirementConfidence::Declared,
        )?,
    }])
}

fn dependency_locks(
    repo_root: &Path,
    owner: &Path,
    target: &Path,
    workspace: Option<&Workspace>,
) -> Result<Vec<DependencyLockRequirement>, AdapterError> {
    let mut paths = BTreeSet::from([target.join("go.mod"), target.join("go.sum")]);
    let target_module = parse_module(repo_root, &target.join("go.mod"))?;
    for replacement in target_module.local_replacements {
        paths.insert(replacement.join("go.mod"));
        paths.insert(replacement.join("go.sum"));
    }
    if owner != target {
        paths.insert(owner.join("go.work"));
        paths.insert(owner.join("go.work.sum"));
        let workspace = workspace.expect("workspace owner");
        for module in &workspace.modules {
            paths.insert(module.join("go.mod"));
            paths.insert(module.join("go.sum"));
        }
        for replacement in &workspace.local_replacements {
            paths.insert(replacement.join("go.mod"));
            paths.insert(replacement.join("go.sum"));
        }
    }
    paths
        .into_iter()
        .filter_map(
            |path| match read_optional_contained_bytes(repo_root, &path).map_err(error) {
                Ok(Some(bytes)) => Some(Ok((path, bytes))),
                Ok(None) => None,
                Err(cause) => Some(Err(cause)),
            },
        )
        .map(|result| {
            result.and_then(|(path, bytes)| {
                let relative_path = relative(repo_root, &path)?;
                Ok(DependencyLockRequirement {
                    path: relative_path.clone(),
                    digest: format!("sha256:{:x}", Sha256::digest(&bytes)),
                    owner_root: relative(repo_root, repo_root)?,
                    source: source(
                        repo_root,
                        if relative_path.ends_with(".sum") {
                            "go_checksum_lock"
                        } else {
                            "go_module_input"
                        },
                        &path,
                        None,
                        RequirementConfidence::Exact,
                    )?,
                })
            })
        })
        .collect()
}

fn relative(repo_root: &Path, path: &Path) -> Result<String, AdapterError> {
    repository_relative(repo_root, path).map_err(error)
}
fn source(
    repo_root: &Path,
    kind: &str,
    path: &Path,
    detail: Option<&str>,
    confidence: RequirementConfidence,
) -> Result<RequirementSource, AdapterError> {
    RequirementSource::new(kind, relative(repo_root, path)?, detail, confidence).map_err(plan_error)
}
fn error(message: impl Into<String>) -> AdapterError {
    AdapterError::new(Language::Go, message)
}
fn plan_error(cause: ayni_core::EnvironmentPlanError) -> AdapterError {
    error(cause.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ayni_adapters_common::environment::assert_environment_capability_conformance;
    use ayni_core::{
        Architecture, EnvironmentDiscoveryRequest, Libc, OperatingSystem, TargetIdentity,
        TargetPlatform,
    };
    use std::fs;
    use tempfile::TempDir;

    fn request(
        repo: &TempDir,
        root: &str,
        signals: Vec<SignalKind>,
    ) -> EnvironmentDiscoveryRequest {
        EnvironmentDiscoveryRequest::new(
            repo.path().to_path_buf(),
            TargetIdentity::new(Language::Go, root).expect("target"),
            signals,
            vec![TargetPlatform {
                os: OperatingSystem::Linux,
                architecture: Architecture::Amd64,
                libc: Libc::Glibc,
            }],
        )
        .expect("request")
    }
    fn module(dir: &Path, name: &str, go: &str) {
        fs::create_dir_all(dir).expect("dir");
        fs::write(dir.join("go.mod"), format!("module {name}\n\ngo {go}\n")).expect("mod");
        fs::write(dir.join("go.sum"), "example v1.0.0 h1:abc\n").expect("sum");
    }

    #[test]
    fn digests_inputs_with_sha256() {
        assert_eq!(
            format!("{:x}", Sha256::digest(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn discovers_workspace_ownership_runtime_tools_and_digests_deterministically() {
        let repo = TempDir::new().expect("repo");
        module(&repo.path().join("api"), "example.com/api", "1.22");
        fs::write(repo.path().join("go.work"), "go 1.22\nuse ./api\n").expect("work");
        let contribution = assert_environment_capability_conformance(
            &GoEnvironmentCapability,
            &request(&repo, "api", vec![SignalKind::Complexity]),
        )
        .expect("contribution");
        let target = contribution.target();
        assert_eq!(target.workspace.as_deref(), Some("."));
        assert_eq!(target.package.as_deref(), Some("example.com/api"));
        assert!(
            matches!(target.runtimes[0].version, VersionRequirement::Minimum { ref version } if version == "1.22")
        );
        assert_eq!(target.signal_tools[0].tool, "gocyclo");
        assert_eq!(
            target.signal_tools[0].scope,
            ToolInstallationScope::Isolated
        );
        assert_eq!(
            target
                .dependency_locks
                .iter()
                .map(|lock| lock.path.as_str())
                .collect::<Vec<_>>(),
            ["api/go.sum", "api/go.mod", "go.work"]
        );
    }

    #[test]
    fn warns_when_a_containing_workspace_does_not_own_the_target() {
        let repo = TempDir::new().expect("repo");
        module(&repo.path().join("api"), "example.com/api", "1.22");
        module(&repo.path().join("other"), "example.com/other", "1.22");
        fs::write(repo.path().join("go.work"), "go 1.22\nuse ./other\n").expect("work");
        let contribution = GoEnvironmentCapability
            .discover(&request(&repo, "api", vec![]))
            .expect("contribution");
        assert!(
            contribution
                .warnings()
                .iter()
                .any(|warning| warning.code == "go_workspace_target_unlisted")
        );
        assert!(contribution.target().workspace.is_none());
    }

    #[test]
    fn reports_conflicting_selectors_in_stable_order_and_rejects_malformed_inputs() {
        let repo = TempDir::new().expect("repo");
        module(repo.path(), "example.com/root", "1.22");
        fs::write(repo.path().join(".go-version"), "1.22.1\n").expect("selector");
        fs::write(repo.path().join(".tool-versions"), "golang 1.23.0\n").expect("selector");
        let contribution = GoEnvironmentCapability
            .discover(&request(&repo, ".", vec![]))
            .expect("contribution");
        assert_eq!(
            contribution.conflicts()[0].code,
            "go_runtime_requirement_conflict"
        );
        assert_eq!(contribution.conflicts()[0].sources.len(), 2);
        fs::write(
            repo.path().join("go.mod"),
            "module example.com/root\ngo 1.22\ngo 1.23\n",
        )
        .expect("bad mod");
        assert!(
            GoEnvironmentCapability
                .discover(&request(&repo, ".", vec![]))
                .is_err()
        );
    }

    #[test]
    fn tracks_contained_local_replacements_and_rejects_escaping_ones() {
        let fixture = TempDir::new().expect("fixture");
        let repo = fixture.path().join("repo");
        let outside = fixture.path().join("outside");
        module(&repo.join("app"), "example.com/app", "1.22");
        module(&repo.join("shared"), "example.com/shared", "1.22");
        module(&outside, "example.com/outside", "1.22");
        fs::write(
            repo.join("app/go.mod"),
            "module example.com/app\n\ngo 1.22\nreplace example.com/shared => ../shared\n",
        )
        .expect("module");
        let contribution = GoEnvironmentCapability
            .discover(
                &EnvironmentDiscoveryRequest::new(
                    repo.clone(),
                    TargetIdentity::new(Language::Go, "app").expect("target"),
                    [],
                    vec![TargetPlatform {
                        os: OperatingSystem::Linux,
                        architecture: Architecture::Amd64,
                        libc: Libc::Glibc,
                    }],
                )
                .expect("request"),
            )
            .expect("plan");
        assert!(
            contribution
                .target()
                .dependency_locks
                .iter()
                .any(|input| input.path == "shared/go.mod")
        );
        fs::write(
            repo.join("app/go.mod"),
            "module example.com/app\n\ngo 1.22\nreplace example.com/outside => ../../outside\n",
        )
        .expect("module");
        let request = EnvironmentDiscoveryRequest::new(
            repo,
            TargetIdentity::new(Language::Go, "app").expect("target"),
            [],
            vec![TargetPlatform {
                os: OperatingSystem::Linux,
                architecture: Architecture::Amd64,
                libc: Libc::Glibc,
            }],
        )
        .expect("request");
        assert!(GoEnvironmentCapability.discover(&request).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_workspace_use_path_that_escapes_repository() {
        use std::os::unix::fs::symlink;
        let fixture = TempDir::new().expect("fixture");
        let repo = fixture.path().join("repo");
        let outside = fixture.path().join("outside");
        fs::create_dir(&repo).expect("repo");
        module(&outside, "example.com/outside", "1.22");
        symlink(&outside, repo.join("outside")).expect("link");
        module(&repo.join("api"), "example.com/api", "1.22");
        fs::write(repo.join("go.work"), "go 1.22\nuse ./outside\n").expect("work");
        let request = EnvironmentDiscoveryRequest::new(
            repo,
            TargetIdentity::new(Language::Go, "api").expect("target"),
            [],
            vec![TargetPlatform {
                os: OperatingSystem::Linux,
                architecture: Architecture::Amd64,
                libc: Libc::Glibc,
            }],
        )
        .expect("request");
        assert!(GoEnvironmentCapability.discover(&request).is_err());
    }
}
