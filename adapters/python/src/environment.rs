//! Read-only Python managed-environment discovery. Only uv projects are portable.
use ayni_adapters_common::repository::{
    read_contained_string, read_optional_contained_bytes, repository_relative,
};
use ayni_core::{
    AdapterError, DependencyLockRequirement, EnvironmentCapability, EnvironmentConflict,
    EnvironmentContribution, EnvironmentDiscoveryRequest, Language, PackageManagerRequirement,
    ProvisioningSupport, RequirementConfidence, RequirementSource, RuntimeRequirement,
    SignalToolRequirement, TargetEnvironment, ToolInstallationScope, VersionRequirement,
    sha256_fingerprint,
};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
#[derive(Debug, Default)]
pub(crate) struct PythonEnvironmentCapability;
impl EnvironmentCapability for PythonEnvironmentCapability {
    fn language(&self) -> Language {
        Language::Python
    }
    fn discover(
        &self,
        request: &EnvironmentDiscoveryRequest,
    ) -> Result<EnvironmentContribution, AdapterError> {
        let target = request.target_root();
        let owner = owner(request.repo_root(), &target)?;
        let manifest = read_toml(request.repo_root(), &target.join("pyproject.toml"))?;
        let owner_manifest = if target == owner {
            manifest.clone()
        } else {
            read_toml(request.repo_root(), &owner.join("pyproject.toml"))?
        };
        let manager = manager(request, &owner)?;
        let (runtime, mut conflicts) =
            runtime(request, &target, &manifest, &owner, &owner_manifest)?;
        conflicts.extend(package_manager_conflicts(request, &owner, &manager)?);
        let (dependency_locks, signal_tools) = portable_inputs(request, &owner, &target, &manager)?;
        EnvironmentContribution::new(
            TargetEnvironment {
                target: request.target().clone(),
                workspace: (owner != target)
                    .then(|| rel(request.repo_root(), &owner))
                    .transpose()?,
                package: manifest
                    .get("project")
                    .and_then(|value| value.get("name"))
                    .and_then(toml::Value::as_str)
                    .map(str::to_owned),
                runtimes: vec![runtime],
                package_manager: Some(manager),
                signal_tools,
                system_requirements: Vec::new(),
                dependency_locks,
            },
            Vec::new(),
            conflicts,
        )
        .map_err(error)
    }
}

const MANAGER_MARKERS: [(&str, &str); 6] = [
    ("uv", "uv.lock"),
    ("poetry", "poetry.lock"),
    ("pdm", "pdm.lock"),
    ("pipenv", "Pipfile.lock"),
    ("hatch", "hatch.toml"),
    ("pip", "requirements.txt"),
];

fn package_manager_conflicts(
    request: &EnvironmentDiscoveryRequest,
    owner: &Path,
    manager: &PackageManagerRequirement,
) -> Result<Vec<EnvironmentConflict>, AdapterError> {
    let markers = MANAGER_MARKERS
        .into_iter()
        .filter(|(_, marker)| owner.join(marker).is_file())
        .collect::<Vec<_>>();
    let mut conflicts = Vec::new();
    if markers.len() > 1 {
        conflicts.push(EnvironmentConflict {
            code: "python_package_manager_conflict".into(),
            message: format!(
                "Python package-manager markers disagree: {}",
                markers
                    .iter()
                    .map(|(family, _)| *family)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            target: Some(request.target().clone()),
            sources: markers
                .iter()
                .map(|(family, marker)| {
                    source(
                        request.repo_root(),
                        &owner.join(marker),
                        "python_package_manager",
                        Some(*family),
                        RequirementConfidence::Declared,
                    )
                })
                .collect::<Result<Vec<_>, _>>()?,
        });
    }
    if manager.family != "uv" || !owner.join("uv.lock").is_file() {
        conflicts.push(EnvironmentConflict {
            code: "python_managed_environment_unsupported".into(),
            message: "Python managed environments currently support only uv-locked pyproject projects (pyproject.toml plus uv.lock); host execution remains available.".into(),
            target: Some(request.target().clone()),
            sources: vec![manager.source.clone()],
        });
    }
    Ok(conflicts)
}

fn portable_inputs(
    request: &EnvironmentDiscoveryRequest,
    owner: &Path,
    target: &Path,
    manager: &PackageManagerRequirement,
) -> Result<(Vec<DependencyLockRequirement>, Vec<SignalToolRequirement>), AdapterError> {
    if manager.family != "uv" || !owner.join("uv.lock").is_file() {
        return Ok((Vec::new(), Vec::new()));
    }
    validate_uv_lock(request.repo_root(), owner)?;
    let dependency_locks = inputs(request.repo_root(), owner, target)?;
    let signal_tools = tools(request, owner, &dependency_locks)?;
    Ok((dependency_locks, signal_tools))
}

fn owner(repo: &Path, target: &Path) -> Result<PathBuf, AdapterError> {
    let mut current = Some(target);
    while let Some(root) = current {
        if !root.starts_with(repo) {
            break;
        }
        if root.join("uv.lock").is_file()
            && (root == target || uv_workspace_contains(repo, root, target)?)
        {
            return Ok(root.to_path_buf());
        }
        current = root.parent().filter(|parent| parent.starts_with(repo));
    }
    Ok(target.to_path_buf())
}

fn uv_workspace_contains(repo: &Path, root: &Path, target: &Path) -> Result<bool, AdapterError> {
    let manifest = read_toml(repo, &root.join("pyproject.toml"))?;
    let Some(workspace) = manifest
        .get("tool")
        .and_then(|tool| tool.get("uv"))
        .and_then(|uv| uv.get("workspace"))
    else {
        return Ok(false);
    };
    let relative = target
        .strip_prefix(root)
        .map_err(|_| error("Python target escapes candidate uv workspace"))?
        .to_string_lossy()
        .replace('\\', "/");
    let patterns = |field: &str| -> Result<Vec<glob::Pattern>, AdapterError> {
        let Some(values) = workspace.get(field) else {
            return Ok(Vec::new());
        };
        values
            .as_array()
            .ok_or_else(|| error(format!("tool.uv.workspace.{field} must be an array")))?
            .iter()
            .map(|value| {
                let value = value.as_str().ok_or_else(|| {
                    error(format!("tool.uv.workspace.{field} must contain strings"))
                })?;
                glob::Pattern::new(value).map_err(|cause| {
                    error(format!(
                        "invalid uv workspace {field} pattern {value}: {cause}"
                    ))
                })
            })
            .collect()
    };
    let members = patterns("members")?;
    let exclude = patterns("exclude")?;
    Ok(members.iter().any(|pattern| pattern.matches(&relative))
        && !exclude.iter().any(|pattern| pattern.matches(&relative)))
}
fn manager(
    r: &EnvironmentDiscoveryRequest,
    owner: &Path,
) -> Result<PackageManagerRequirement, AdapterError> {
    let (family, marker) = [
        ("uv", "uv.lock"),
        ("poetry", "poetry.lock"),
        ("pdm", "pdm.lock"),
        ("pipenv", "Pipfile.lock"),
        ("hatch", "hatch.toml"),
        ("pip", "requirements.txt"),
    ]
    .into_iter()
    .find(|(_, m)| owner.join(m).is_file())
    .unwrap_or(("pip", "pyproject.toml"));
    let version = if family == "uv" {
        let manifest = read_toml(r.repo_root(), &owner.join("pyproject.toml"))?;
        manifest
            .get("tool")
            .and_then(|tool| tool.get("uv"))
            .and_then(|uv| uv.get("required-version"))
            .and_then(toml::Value::as_str)
            .map(req)
            .transpose()?
            .unwrap_or(
                VersionRequirement::unresolved(
                    "uv managed environments require tool.uv.required-version",
                )
                .map_err(error)?,
            )
    } else {
        VersionRequirement::unresolved("managed Python support requires uv.lock").map_err(error)?
    };
    Ok(PackageManagerRequirement {
        family: family.into(),
        version,
        ownership_root: rel(r.repo_root(), owner)?,
        source: if family == "uv" {
            source(
                r.repo_root(),
                &owner.join("pyproject.toml"),
                "python_uv_required_version",
                Some(family),
                RequirementConfidence::Declared,
            )?
        } else {
            source(
                r.repo_root(),
                &owner.join(marker),
                "python_package_manager",
                Some(family),
                RequirementConfidence::Declared,
            )?
        },
    })
}
type PythonRuntimeSelection = (
    VersionRequirement,
    RequirementSource,
    Vec<EnvironmentConflict>,
);

fn runtime(
    request: &EnvironmentDiscoveryRequest,
    target: &Path,
    target_manifest: &toml::Value,
    owner: &Path,
    owner_manifest: &toml::Value,
) -> Result<(RuntimeRequirement, Vec<EnvironmentConflict>), AdapterError> {
    let selector = runtime_selector(request.repo_root(), target, owner)?;
    let compatibility = requires(target_manifest).or_else(|| {
        (owner != target)
            .then(|| requires(owner_manifest))
            .flatten()
    });
    let declared_manifest = if owner != target {
        owner.join("pyproject.toml")
    } else {
        target.join("pyproject.toml")
    };
    let (version, source, conflicts) =
        select_python_runtime(request, target, selector, compatibility, &declared_manifest)?;
    Ok((
        RuntimeRequirement {
            runtime: "python".into(),
            version,
            components: Vec::new(),
            targets: Vec::new(),
            source,
        },
        conflicts,
    ))
}

fn runtime_selector(
    repo: &Path,
    target: &Path,
    owner: &Path,
) -> Result<Option<(String, RequirementSource)>, AdapterError> {
    if let Some(selector) = selector(repo, target)? {
        return Ok(Some(selector));
    }
    if owner != target {
        return selector(repo, owner);
    }
    Ok(None)
}

fn select_python_runtime(
    request: &EnvironmentDiscoveryRequest,
    target: &Path,
    selector: Option<(String, RequirementSource)>,
    compatibility: Option<String>,
    declared_manifest: &Path,
) -> Result<PythonRuntimeSelection, AdapterError> {
    match (selector, compatibility) {
        (Some((selected, selected_source)), compatibility) => selected_python_runtime(
            request,
            selected,
            selected_source,
            compatibility.as_deref(),
            declared_manifest,
        ),
        (None, Some(compatibility)) => {
            compatibility_python_runtime(request.repo_root(), compatibility, declared_manifest)
        }
        (None, None) => Ok((
            VersionRequirement::unresolved("no .python-version or project.requires-python")
                .map_err(error)?,
            source(
                request.repo_root(),
                &target.join("pyproject.toml"),
                "python_runtime_unresolved",
                None,
                RequirementConfidence::Assumed,
            )?,
            Vec::new(),
        )),
    }
}

fn selected_python_runtime(
    request: &EnvironmentDiscoveryRequest,
    selected: String,
    selected_source: RequirementSource,
    compatibility: Option<&str>,
    declared_manifest: &Path,
) -> Result<PythonRuntimeSelection, AdapterError> {
    let conflict = match (exact_python_version(&selected)?, compatibility) {
        (Some(version), Some(expression))
            if !python_requirement_satisfied(version, expression)? =>
        {
            Some(EnvironmentConflict {
                code: "python_runtime_source_conflict".into(),
                message: format!(
                    ".python-version {selected} does not satisfy requires-python {expression}"
                ),
                target: Some(request.target().clone()),
                sources: vec![
                    selected_source.clone(),
                    source(
                        request.repo_root(),
                        declared_manifest,
                        "python_requires_python",
                        Some(expression),
                        RequirementConfidence::Declared,
                    )?,
                ],
            })
        }
        _ => None,
    };
    Ok((
        req(&selected)?,
        selected_source,
        conflict.into_iter().collect(),
    ))
}

fn compatibility_python_runtime(
    repo: &Path,
    compatibility: String,
    declared_manifest: &Path,
) -> Result<PythonRuntimeSelection, AdapterError> {
    validate_python_requirement(&compatibility)?;
    Ok((
        VersionRequirement::compatibility(&compatibility).map_err(error)?,
        source(
            repo,
            declared_manifest,
            "python_requires_python",
            Some(&compatibility),
            RequirementConfidence::Declared,
        )?,
        Vec::new(),
    ))
}

fn selector(repo: &Path, root: &Path) -> Result<Option<(String, RequirementSource)>, AdapterError> {
    let p = root.join(".python-version");
    let Some(b) = read_optional_contained_bytes(repo, &p).map_err(error)? else {
        return Ok(None);
    };
    let x = String::from_utf8(b).map_err(|_| error(".python-version is not UTF-8"))?;
    let x = x.trim();
    if x.is_empty() || x.lines().count() != 1 {
        return Err(error(".python-version must contain one non-empty selector"));
    }
    Ok(Some((
        x.into(),
        source(
            repo,
            &p,
            "python_version_file",
            Some(x),
            RequirementConfidence::Declared,
        )?,
    )))
}
fn requires(v: &toml::Value) -> Option<String> {
    v.get("project")?
        .get("requires-python")?
        .as_str()
        .map(str::to_owned)
}

pub(crate) type PythonVersion = [u64; 3];

pub(crate) fn exact_python_version(value: &str) -> Result<Option<PythonVersion>, AdapterError> {
    let parts = value.split('.').collect::<Vec<_>>();
    if parts.len() != 3 {
        return Ok(None);
    }
    let mut version = [0_u64; 3];
    for (index, part) in parts.into_iter().enumerate() {
        version[index] = part
            .parse()
            .map_err(|_| error(format!("invalid Python version {value}")))?;
    }
    Ok(Some(version))
}

pub(crate) fn parse_python_release(value: &str) -> Result<PythonVersion, AdapterError> {
    let parts = value.split('.').collect::<Vec<_>>();
    if !(2..=3).contains(&parts.len()) {
        return Err(error(format!(
            "unsupported Python version requirement {value}"
        )));
    }
    let mut version = [0_u64; 3];
    for (index, part) in parts.into_iter().enumerate() {
        version[index] = part
            .parse()
            .map_err(|_| error(format!("invalid Python version requirement {value}")))?;
    }
    Ok(version)
}

fn validate_python_requirement(expression: &str) -> Result<(), AdapterError> {
    python_requirement_satisfied([0, 0, 0], expression).map(|_| ())
}

pub(crate) fn python_requirement_satisfied(
    version: PythonVersion,
    expression: &str,
) -> Result<bool, AdapterError> {
    let clauses = expression
        .split(',')
        .map(str::trim)
        .filter(|clause| !clause.is_empty())
        .map(parse_python_clause)
        .collect::<Result<Vec<_>, _>>()?;
    if clauses.is_empty() {
        return Err(error("requires-python must not be empty"));
    }
    Ok(clauses
        .iter()
        .all(|clause| python_clause_satisfied(version, clause)))
}

#[derive(Debug, Clone, Copy)]
enum PythonOperator {
    GreaterEqual,
    LessEqual,
    Equal,
    Compatible,
    Greater,
    Less,
}

struct PythonClause {
    operator: PythonOperator,
    required: PythonVersion,
    wildcard: bool,
    precision: usize,
}

fn parse_python_clause(clause: &str) -> Result<PythonClause, AdapterError> {
    let (operator, value) = [
        (">=", PythonOperator::GreaterEqual),
        ("<=", PythonOperator::LessEqual),
        ("==", PythonOperator::Equal),
        ("~=", PythonOperator::Compatible),
        (">", PythonOperator::Greater),
        ("<", PythonOperator::Less),
    ]
    .into_iter()
    .find_map(|(prefix, operator)| clause.strip_prefix(prefix).map(|value| (operator, value)))
    .ok_or_else(|| error(format!("unsupported requires-python clause {clause}")))?;
    let wildcard = value.ends_with(".*");
    let value = value.strip_suffix(".*").unwrap_or(value);
    Ok(PythonClause {
        operator,
        required: parse_python_release(value)?,
        wildcard,
        precision: value.split('.').count(),
    })
}

fn python_clause_satisfied(version: PythonVersion, clause: &PythonClause) -> bool {
    match clause.operator {
        PythonOperator::GreaterEqual => version >= clause.required,
        PythonOperator::LessEqual => version <= clause.required,
        PythonOperator::Greater => version > clause.required,
        PythonOperator::Less => version < clause.required,
        PythonOperator::Equal if clause.wildcard => {
            version[0] == clause.required[0] && version[1] == clause.required[1]
        }
        PythonOperator::Equal => version == clause.required,
        PythonOperator::Compatible => {
            version >= clause.required && version < compatible_upper_bound(clause)
        }
    }
}

fn compatible_upper_bound(clause: &PythonClause) -> PythonVersion {
    if clause.precision == 2 {
        [clause.required[0] + 1, 0, 0]
    } else {
        [clause.required[0], clause.required[1] + 1, 0]
    }
}

fn req(v: &str) -> Result<VersionRequirement, AdapterError> {
    let p: Vec<_> = v.split('.').collect();
    if p.len() == 3
        && p.iter()
            .all(|x| !x.is_empty() && x.bytes().all(|b| b.is_ascii_digit()))
    {
        VersionRequirement::exact(v).map_err(error)
    } else {
        VersionRequirement::selector(v).map_err(error)
    }
}
fn inputs(
    repo: &Path,
    owner: &Path,
    target: &Path,
) -> Result<Vec<DependencyLockRequirement>, AdapterError> {
    let mut ps = BTreeSet::new();
    ps.insert(owner.join("pyproject.toml"));
    ps.insert(owner.join("uv.lock"));
    ps.insert(target.join("pyproject.toml"));
    for entry in walkdir::WalkDir::new(owner)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| {
            !matches!(
                entry.file_name().to_str(),
                Some(".git" | ".ayni" | ".venv" | "venv" | "__pycache__")
            )
        })
    {
        let entry =
            entry.map_err(|cause| error(format!("failed to inspect uv workspace: {cause}")))?;
        if entry.file_type().is_file() && entry.file_name() == "pyproject.toml" {
            let root = entry.path().parent().unwrap_or(owner);
            if root == owner || uv_workspace_contains(repo, owner, root)? {
                ps.insert(entry.into_path());
            }
        }
    }
    let own = rel(repo, owner)?;
    ps.into_iter()
        .map(|p| {
            let b = read_optional_contained_bytes(repo, &p)
                .map_err(error)?
                .ok_or_else(|| error(format!("missing managed input {}", p.display())))?;
            if b.is_empty() {
                return Err(error(format!("managed input {} is empty", p.display())));
            }
            Ok(DependencyLockRequirement {
                path: rel(repo, &p)?,
                digest: sha256_fingerprint(b),
                owner_root: own.clone(),
                source: source(
                    repo,
                    &p,
                    if p.ends_with("uv.lock") {
                        "uv_lock"
                    } else {
                        "python_manifest"
                    },
                    None,
                    RequirementConfidence::Exact,
                )?,
            })
        })
        .collect()
}
fn validate_uv_lock(repo: &Path, owner: &Path) -> Result<(), AdapterError> {
    let path = owner.join("uv.lock");
    toml::from_str::<toml::Value>(&read_contained_string(repo, &path).map_err(error)?)
        .map(|_| ())
        .map_err(|cause| error(format!("failed to parse {}: {cause}", path.display())))
}

fn tools(
    r: &EnvironmentDiscoveryRequest,
    owner: &Path,
    locks: &[DependencyLockRequirement],
) -> Result<Vec<SignalToolRequirement>, AdapterError> {
    let p = owner.join("uv.lock");
    let v: toml::Value = toml::from_str(&read_contained_string(r.repo_root(), &p).map_err(error)?)
        .map_err(|e| error(format!("failed to parse {}: {e}", p.display())))?;
    let mut declared =
        declared_dependencies(&read_toml(r.repo_root(), &owner.join("pyproject.toml"))?)?;
    let target_manifest = r.target_root().join("pyproject.toml");
    if target_manifest != owner.join("pyproject.toml") {
        declared.extend(declared_dependencies(&read_toml(
            r.repo_root(),
            &target_manifest,
        )?)?);
    }
    let l = locks
        .iter()
        .find(|x| x.path.ends_with("uv.lock"))
        .ok_or_else(|| error("uv.lock missing from dependency inputs"))?;
    // The Python runtime is modeled separately. Derive project tools from the
    // catalog so managed tool requirements cannot diverge from collectors.
    let w = crate::catalog::PYTHON_CATALOG
        .iter()
        .filter(|entry| entry.name != "python" && r.requires_any(entry.for_signals))
        .map(|entry| (entry.name, entry.for_signals))
        .collect::<Vec<_>>();
    w.into_iter()
        .map(|(n, signals)| {
            if !declared.contains(n) {
                return Err(error(format!(
                    "{n} is required by an enabled signal but is not declared by the uv project"
                )));
            }
            let ver = locked(&v, n)?
                .ok_or_else(|| error(format!("{n} is not an exact dependency in uv.lock")))?;
            Ok(SignalToolRequirement {
                tool: n.into(),
                version: VersionRequirement::exact(ver).map_err(error)?,
                provider: "uv_locked_project_dependency".into(),
                scope: ToolInstallationScope::Project,
                signals: signals.to_vec(),
                supported_platforms: r.requested_platforms().to_vec(),
                provisioning: ProvisioningSupport::LockedOffline,
                modifies_checkout: false,
                source: RequirementSource::new(
                    "uv_lock_tool",
                    l.path.clone(),
                    Some(n),
                    RequirementConfidence::Exact,
                )
                .map_err(error)?,
            })
        })
        .collect()
}
fn declared_dependencies(value: &toml::Value) -> Result<BTreeSet<String>, AdapterError> {
    let mut requirements = Vec::new();
    if let Some(values) = value
        .get("project")
        .and_then(|project| project.get("dependencies"))
    {
        requirements.extend(requirement_array(values, "project.dependencies")?);
    }
    if let Some(groups) = value
        .get("project")
        .and_then(|project| project.get("optional-dependencies"))
        .and_then(toml::Value::as_table)
    {
        for (name, values) in groups {
            requirements.extend(requirement_array(
                values,
                &format!("project.optional-dependencies.{name}"),
            )?);
        }
    }
    if let Some(groups) = value
        .get("dependency-groups")
        .and_then(toml::Value::as_table)
    {
        for (name, values) in groups {
            requirements.extend(requirement_array(
                values,
                &format!("dependency-groups.{name}"),
            )?);
        }
    }
    if let Some(values) = value
        .get("tool")
        .and_then(|tool| tool.get("uv"))
        .and_then(|uv| uv.get("dev-dependencies"))
    {
        requirements.extend(requirement_array(values, "tool.uv.dev-dependencies")?);
    }
    requirements.into_iter().map(requirement_name).collect()
}

fn requirement_array(value: &toml::Value, field: &str) -> Result<Vec<String>, AdapterError> {
    value
        .as_array()
        .ok_or_else(|| error(format!("{field} must be an array")))?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| error(format!("{field} must contain strings")))
        })
        .collect()
}

fn requirement_name(requirement: String) -> Result<String, AdapterError> {
    let name = requirement
        .split(['[', '<', '>', '=', '!', '~', ';', ' '])
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
        .replace('_', "-");
    if name.is_empty()
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.'))
    {
        Err(error(format!(
            "invalid Python dependency declaration {requirement}"
        )))
    } else {
        Ok(name)
    }
}

fn locked<'a>(value: &'a toml::Value, name: &str) -> Result<Option<&'a str>, AdapterError> {
    let versions = value
        .get("package")
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter(|package| package.get("name").and_then(toml::Value::as_str) == Some(name))
        .filter_map(|package| package.get("version").and_then(toml::Value::as_str))
        .collect::<BTreeSet<_>>();
    if versions.len() > 1 {
        Err(error(format!(
            "uv.lock resolves multiple versions of {name}; platform applicability is ambiguous"
        )))
    } else {
        Ok(versions.into_iter().next())
    }
}

fn read_toml(repo: &Path, p: &Path) -> Result<toml::Value, AdapterError> {
    toml::from_str(&read_contained_string(repo, p).map_err(error)?)
        .map_err(|e| error(format!("failed to parse {}: {e}", p.display())))
}
fn rel(repo: &Path, p: &Path) -> Result<String, AdapterError> {
    repository_relative(repo, p).map_err(error)
}
fn source(
    repo: &Path,
    p: &Path,
    k: &str,
    d: Option<&str>,
    c: RequirementConfidence,
) -> Result<RequirementSource, AdapterError> {
    RequirementSource::new(k, rel(repo, p)?, d, c).map_err(error)
}
fn error(e: impl std::fmt::Display) -> AdapterError {
    AdapterError::new(Language::Python, e.to_string())
}
#[cfg(test)]
mod tests;
