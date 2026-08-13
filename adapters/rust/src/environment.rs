//! Read-only Rust environment discovery.

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
use std::path::{Path, PathBuf};

#[derive(Debug, Default)]
pub struct RustEnvironmentCapability;

impl EnvironmentCapability for RustEnvironmentCapability {
    fn language(&self) -> Language {
        Language::Rust
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
    let ownership = discover_ownership(request.repo_root(), &target_root)?;
    let (_toolchain_root, toml_toolchain, legacy_toolchain) =
        discover_toolchain(request.repo_root(), &target_root)?;
    let conflicts = toolchain_conflicts(request, &toml_toolchain, &legacy_toolchain);
    let (mut runtime, warnings) =
        runtime_requirement(request, &ownership, toml_toolchain.or(legacy_toolchain))?;
    add_coverage_component(request, &mut runtime);

    EnvironmentContribution::new(
        TargetEnvironment {
            target: request.target().clone(),
            workspace: ownership.workspace_path,
            package: ownership.package_name,
            signal_tools: signal_tools(request, &ownership.manifest_path, &runtime)?,
            runtimes: vec![runtime],
            package_manager: None,
            system_requirements: Vec::new(),
            dependency_locks: dependency_locks(request.repo_root(), &ownership.workspace_root)?,
        },
        warnings,
        conflicts,
    )
    .map_err(plan_error)
}

fn toolchain_conflicts(
    request: &EnvironmentDiscoveryRequest,
    toml: &Option<Toolchain>,
    legacy: &Option<Toolchain>,
) -> Vec<EnvironmentConflict> {
    match (toml, legacy) {
        (Some(toml), Some(legacy)) if !toolchain_equivalent(toml, legacy) => {
            vec![EnvironmentConflict {
                code: String::from("rust_toolchain_source_conflict"),
                message: String::from(
                    "rust-toolchain.toml and rust-toolchain disagree; remove or align the legacy selector",
                ),
                target: Some(request.target().clone()),
                sources: vec![toml.source.clone(), legacy.source.clone()],
            }]
        }
        _ => Vec::new(),
    }
}

fn runtime_requirement(
    request: &EnvironmentDiscoveryRequest,
    ownership: &Ownership,
    toolchain: Option<Toolchain>,
) -> Result<(RuntimeRequirement, Vec<EnvironmentWarning>), AdapterError> {
    if let Some(toolchain) = toolchain {
        return Ok((
            RuntimeRequirement {
                runtime: String::from("rust"),
                version: version_requirement(&toolchain.channel)?,
                components: toolchain.components,
                targets: toolchain.targets,
                source: toolchain.source,
            },
            Vec::new(),
        ));
    }
    if let Some(rust_version) = &ownership.rust_version {
        return Ok((
            RuntimeRequirement {
                runtime: String::from("rust"),
                version: VersionRequirement::minimum(&rust_version.version).map_err(plan_error)?,
                components: Vec::new(),
                targets: Vec::new(),
                source: rust_version.source.clone(),
            },
            Vec::new(),
        ));
    }

    Ok((
        RuntimeRequirement {
            runtime: String::from("rust"),
            version: VersionRequirement::unresolved("no Rust toolchain or Cargo rust-version")
                .map_err(plan_error)?,
            components: Vec::new(),
            targets: Vec::new(),
            source: source(
                "unresolved",
                &ownership.manifest_path,
                None,
                RequirementConfidence::Assumed,
            )?,
        },
        vec![EnvironmentWarning {
            code: String::from("rust_runtime_unresolved"),
            message: String::from(
                "No Rust toolchain or Cargo rust-version was found; add rust-toolchain.toml or package.rust-version.",
            ),
            target: Some(request.target().clone()),
        }],
    ))
}

fn add_coverage_component(request: &EnvironmentDiscoveryRequest, runtime: &mut RuntimeRequirement) {
    if request.requires_any(&[SignalKind::Coverage])
        && !runtime
            .components
            .iter()
            .any(|component| component == "llvm-tools-preview")
    {
        runtime.components.push(String::from("llvm-tools-preview"));
    }
}

#[derive(Debug)]
struct Ownership {
    workspace_root: PathBuf,
    workspace_path: Option<String>,
    manifest_path: String,
    package_name: Option<String>,
    rust_version: Option<RustVersion>,
}
#[derive(Debug)]
struct RustVersion {
    version: String,
    source: RequirementSource,
}

fn discover_ownership(repo_root: &Path, target_root: &Path) -> Result<Ownership, AdapterError> {
    let target_manifest = target_root.join("Cargo.toml");
    let target_value = read_toml(repo_root, &target_manifest)?;
    let workspace_root = workspace_root(repo_root, target_root)?;
    let workspace_manifest = workspace_root.join("Cargo.toml");
    let workspace_value = if workspace_manifest == target_manifest {
        target_value.clone()
    } else {
        read_toml(repo_root, &workspace_manifest)?
    };
    let manifest_path = relative(repo_root, &target_manifest)?;
    let package = target_value.get("package").and_then(toml::Value::as_table);
    let package_name = package
        .and_then(|p| p.get("name"))
        .and_then(toml::Value::as_str)
        .map(str::to_owned);
    let rust_version = package
        .and_then(|package| package.get("rust-version"))
        .map(|value| {
            rust_version(
                repo_root,
                &manifest_path,
                &workspace_manifest,
                &workspace_value,
                value,
            )
        })
        .transpose()?;
    Ok(Ownership {
        workspace_path: (workspace_root != target_root)
            .then(|| relative(repo_root, &workspace_root))
            .transpose()?,
        workspace_root,
        manifest_path,
        package_name,
        rust_version,
    })
}

fn rust_version(
    repo_root: &Path,
    manifest_path: &str,
    workspace_manifest: &Path,
    workspace_value: &toml::Value,
    value: &toml::Value,
) -> Result<RustVersion, AdapterError> {
    match value {
        toml::Value::String(version) => Ok(RustVersion {
            version: version.clone(),
            source: source(
                "cargo_package_rust_version",
                manifest_path,
                None,
                RequirementConfidence::Declared,
            )?,
        }),
        toml::Value::Table(table)
            if table.len() == 1
                && table.get("workspace").and_then(toml::Value::as_bool) == Some(true) =>
        {
            workspace_rust_version(repo_root, workspace_manifest, workspace_value)
        }
        _ => Err(adapter_error(
            "Cargo package rust-version must be a string or { workspace = true }",
        )),
    }
}

fn workspace_rust_version(
    repo_root: &Path,
    manifest: &Path,
    value: &toml::Value,
) -> Result<RustVersion, AdapterError> {
    let version = value
        .get("workspace")
        .and_then(|workspace| workspace.get("package"))
        .and_then(|package| package.get("rust-version"))
        .and_then(toml::Value::as_str)
        .ok_or_else(|| {
            adapter_error(
                "workspace-inherited Cargo rust-version requires a string at workspace.package.rust-version",
            )
        })?
        .to_owned();
    let path = relative(repo_root, manifest)?;
    Ok(RustVersion {
        version,
        source: source(
            "cargo_workspace_package_rust_version",
            &path,
            None,
            RequirementConfidence::Declared,
        )?,
    })
}

fn workspace_root(repo_root: &Path, target_root: &Path) -> Result<PathBuf, AdapterError> {
    let mut current = target_root;
    loop {
        let manifest = current.join("Cargo.toml");
        if manifest.is_file() && read_toml(repo_root, &manifest)?.get("workspace").is_some() {
            return Ok(current.to_path_buf());
        }
        if current == repo_root {
            return Ok(target_root.to_path_buf());
        }
        current = current
            .parent()
            .filter(|parent| parent.starts_with(repo_root))
            .ok_or_else(|| {
                adapter_error("Rust target has no repository-contained workspace ancestor")
            })?;
    }
}

#[derive(Debug, Clone)]
struct Toolchain {
    channel: String,
    components: Vec<String>,
    targets: Vec<String>,
    source: RequirementSource,
}

fn discover_toolchain(
    repo_root: &Path,
    target_root: &Path,
) -> Result<(PathBuf, Option<Toolchain>, Option<Toolchain>), AdapterError> {
    let mut current = Some(target_root);
    while let Some(root) = current {
        let toml = read_toolchain_toml(repo_root, root)?;
        let legacy = read_legacy_toolchain(repo_root, root)?;
        if toml.is_some() || legacy.is_some() {
            return Ok((root.to_path_buf(), toml, legacy));
        }
        if root == repo_root {
            break;
        }
        current = root.parent().filter(|parent| parent.starts_with(repo_root));
    }
    Ok((target_root.to_path_buf(), None, None))
}

fn read_toolchain_toml(repo_root: &Path, root: &Path) -> Result<Option<Toolchain>, AdapterError> {
    let path = root.join("rust-toolchain.toml");
    let Some(content) = read_optional_contained_string(repo_root, &path).map_err(adapter_error)?
    else {
        return Ok(None);
    };
    let value: toml::Value = toml::from_str(&content)
        .map_err(|error| adapter_error(format!("failed to parse {}: {error}", path.display())))?;
    let toolchain = value
        .get("toolchain")
        .ok_or_else(|| adapter_error(format!("{} must contain [toolchain]", path.display())))?;
    toolchain_from_value(repo_root, &path, "rust_toolchain_toml", toolchain)
}

fn read_legacy_toolchain(repo_root: &Path, root: &Path) -> Result<Option<Toolchain>, AdapterError> {
    let path = root.join("rust-toolchain");
    let Some(content) = read_optional_contained_string(repo_root, &path).map_err(adapter_error)?
    else {
        return Ok(None);
    };
    let channel = content.trim();
    if channel.is_empty() || channel.lines().count() != 1 {
        return Err(adapter_error(format!(
            "{} must contain one non-empty toolchain selector",
            path.display()
        )));
    }
    Ok(Some(Toolchain {
        channel: channel.to_owned(),
        components: Vec::new(),
        targets: Vec::new(),
        source: source(
            "rust_toolchain_legacy",
            &relative(repo_root, &path)?,
            None,
            RequirementConfidence::Declared,
        )?,
    }))
}

fn toolchain_from_value(
    repo_root: &Path,
    path: &Path,
    kind: &str,
    value: &toml::Value,
) -> Result<Option<Toolchain>, AdapterError> {
    let channel = value
        .get("channel")
        .and_then(toml::Value::as_str)
        .ok_or_else(|| {
            adapter_error(format!(
                "{} toolchain.channel must be a string",
                path.display()
            ))
        })?;
    let mut components = strings(value.get("components"), path, "components")?;
    let mut targets = strings(value.get("targets"), path, "targets")?;
    components.sort();
    components.dedup();
    targets.sort();
    targets.dedup();
    Ok(Some(Toolchain {
        channel: channel.to_owned(),
        components,
        targets,
        source: source(
            kind,
            &relative(repo_root, path)?,
            None,
            RequirementConfidence::Declared,
        )?,
    }))
}

fn strings(
    value: Option<&toml::Value>,
    path: &Path,
    field: &str,
) -> Result<Vec<String>, AdapterError> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let values = value.as_array().ok_or_else(|| {
        adapter_error(format!(
            "{} toolchain.{field} must be an array of strings",
            path.display()
        ))
    })?;
    values
        .iter()
        .map(|value| {
            value.as_str().map(str::to_owned).ok_or_else(|| {
                adapter_error(format!(
                    "{} toolchain.{field} must contain only strings",
                    path.display()
                ))
            })
        })
        .collect()
}
fn toolchain_equivalent(left: &Toolchain, right: &Toolchain) -> bool {
    left.channel == right.channel
}
fn version_requirement(value: &str) -> Result<VersionRequirement, AdapterError> {
    let numeric_exact = value.split('.').count() == 3
        && value
            .split('.')
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()));
    let qualified_exact = value.split_once('-').is_some_and(|(base, qualifier)| {
        (!qualifier.is_empty()
            && base.split('.').count() == 3
            && base
                .split('.')
                .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit())))
            || (matches!(base, "nightly" | "beta") && is_iso_date(qualifier))
    });
    if numeric_exact || qualified_exact {
        VersionRequirement::exact(value).map_err(plan_error)
    } else {
        VersionRequirement::selector(value).map_err(plan_error)
    }
}

fn is_iso_date(value: &str) -> bool {
    let parts = value.split('-').collect::<Vec<_>>();
    matches!(parts.as_slice(), [year, month, day]
        if year.len() == 4
            && month.len() == 2
            && day.len() == 2
            && parts.iter().all(|part| part.bytes().all(|byte| byte.is_ascii_digit())))
}

fn signal_tools(
    request: &EnvironmentDiscoveryRequest,
    manifest_path: &str,
    _runtime: &RuntimeRequirement,
) -> Result<Vec<SignalToolRequirement>, AdapterError> {
    let source = source(
        "rust_adapter_catalog",
        manifest_path,
        None,
        RequirementConfidence::Declared,
    )?;
    let platforms = request.requested_platforms().to_vec();
    let mut tools = Vec::new();
    if request.requires_any(&[SignalKind::Coverage]) {
        tools.push(tool(
            "cargo-llvm-cov",
            VersionRequirement::exact("0.8.5").map_err(plan_error)?,
            "cargo-install",
            ToolInstallationScope::Isolated,
            vec![SignalKind::Coverage],
            platforms.clone(),
            source.clone(),
        ));
    }
    if request.requires_any(&[SignalKind::Complexity]) {
        tools.push(tool(
            "rust-code-analysis-cli",
            VersionRequirement::unresolved("catalog does not pin rust-code-analysis-cli")
                .map_err(plan_error)?,
            "cargo-install",
            ToolInstallationScope::Isolated,
            vec![SignalKind::Complexity],
            platforms.clone(),
            source.clone(),
        ));
    }
    if request.requires_any(&[SignalKind::Mutation]) {
        tools.push(tool(
            "cargo-mutants",
            VersionRequirement::unresolved("catalog does not pin cargo-mutants")
                .map_err(plan_error)?,
            "cargo-install",
            ToolInstallationScope::Isolated,
            vec![SignalKind::Mutation],
            platforms,
            source,
        ));
    }
    Ok(tools)
}

fn tool(
    tool: &str,
    version: VersionRequirement,
    provider: &str,
    scope: ToolInstallationScope,
    signals: Vec<SignalKind>,
    supported_platforms: Vec<ayni_core::TargetPlatform>,
    source: RequirementSource,
) -> SignalToolRequirement {
    SignalToolRequirement {
        tool: tool.into(),
        version,
        provider: provider.into(),
        scope,
        signals,
        supported_platforms,
        provisioning: ProvisioningSupport::OnlineOnly,
        modifies_checkout: false,
        source,
    }
}

fn dependency_locks(
    repo_root: &Path,
    owner_root: &Path,
) -> Result<Vec<DependencyLockRequirement>, AdapterError> {
    let lock = owner_root.join("Cargo.lock");
    let Some(content) = read_optional_contained_bytes(repo_root, &lock).map_err(adapter_error)?
    else {
        return Ok(Vec::new());
    };
    let path = relative(repo_root, &lock)?;
    let digest = format!("sha256:{:x}", Sha256::digest(content));
    Ok(vec![DependencyLockRequirement {
        path: path.clone(),
        digest,
        owner_root: relative(repo_root, owner_root)?,
        source: source("cargo_lock", &path, None, RequirementConfidence::Exact)?,
    }])
}
fn read_toml(repo_root: &Path, path: &Path) -> Result<toml::Value, AdapterError> {
    let content = read_contained_string(repo_root, path).map_err(adapter_error)?;
    toml::from_str(&content)
        .map_err(|error| adapter_error(format!("failed to parse {}: {error}", path.display())))
}
fn relative(repo_root: &Path, path: &Path) -> Result<String, AdapterError> {
    repository_relative(repo_root, path).map_err(adapter_error)
}
fn source(
    kind: &str,
    path: &str,
    detail: Option<&str>,
    confidence: RequirementConfidence,
) -> Result<RequirementSource, AdapterError> {
    RequirementSource::new(kind, path, detail, confidence).map_err(plan_error)
}
fn adapter_error(message: impl Into<String>) -> AdapterError {
    AdapterError::new(Language::Rust, message)
}
fn plan_error(error: ayni_core::EnvironmentPlanError) -> AdapterError {
    adapter_error(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::RustEnvironmentCapability;
    use ayni_adapters_common::environment::assert_environment_capability_conformance;
    use ayni_core::{
        Architecture, EnvironmentCapability, EnvironmentDiscoveryRequest, Language, Libc,
        OperatingSystem, SignalKind, TargetIdentity, TargetPlatform, VersionRequirement,
    };
    use std::fs;
    use tempfile::TempDir;

    fn request(
        root: &std::path::Path,
        target: &str,
        signals: Vec<SignalKind>,
    ) -> EnvironmentDiscoveryRequest {
        EnvironmentDiscoveryRequest::new(
            root.to_path_buf(),
            TargetIdentity::new(Language::Rust, target).expect("target"),
            signals,
            vec![TargetPlatform {
                os: OperatingSystem::Linux,
                architecture: Architecture::Amd64,
                libc: Libc::Glibc,
            }],
        )
        .expect("request")
    }
    fn manifest(root: &std::path::Path, content: &str) {
        fs::write(root.join("Cargo.toml"), content).expect("manifest");
    }
    fn discover(
        root: &std::path::Path,
        target: &str,
        signals: Vec<SignalKind>,
    ) -> ayni_core::EnvironmentContribution {
        RustEnvironmentCapability
            .discover(&request(root, target, signals))
            .expect("discovery")
    }

    #[test]
    fn parses_exact_toml_toolchain_and_conforms_without_mutation() {
        let fixture = TempDir::new().expect("fixture");
        manifest(
            fixture.path(),
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\n",
        );
        fs::write(fixture.path().join("rust-toolchain.toml"), "[toolchain]\nchannel = \"1.85.0\"\ncomponents = [\"rustfmt\", \"llvm-tools-preview\"]\ntargets = [\"wasm32-wasip1\"]\n").expect("toolchain");
        let contribution = assert_environment_capability_conformance(
            &RustEnvironmentCapability,
            &request(fixture.path(), ".", vec![]),
        )
        .expect("conformance");
        let runtime = &contribution.target().runtimes[0];
        assert_eq!(
            runtime.version,
            VersionRequirement::exact("1.85.0").expect("version")
        );
        assert_eq!(runtime.components, ["llvm-tools-preview", "rustfmt"]);
        assert_eq!(runtime.targets, ["wasm32-wasip1"]);
        assert_eq!(runtime.source.path, "rust-toolchain.toml");
    }

    #[test]
    fn preserves_legacy_toolchain_selector() {
        let fixture = TempDir::new().expect("fixture");
        manifest(
            fixture.path(),
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\n",
        );
        fs::write(fixture.path().join("rust-toolchain"), "stable\n").expect("toolchain");
        let contribution = discover(fixture.path(), ".", vec![]);
        assert_eq!(
            contribution.target().runtimes[0].version,
            VersionRequirement::selector("stable").expect("selector")
        );
    }

    #[test]
    fn discovers_ancestor_toolchain_and_preserves_pinned_nightly() {
        let fixture = TempDir::new().expect("fixture");
        manifest(fixture.path(), "[workspace]\nmembers = [\"crates/app\"]\n");
        fs::write(
            fixture.path().join("rust-toolchain.toml"),
            "[toolchain]\nchannel = \"nightly-2026-08-01\"\n",
        )
        .expect("toolchain");
        fs::create_dir_all(fixture.path().join("crates/app")).expect("package directory");
        manifest(
            &fixture.path().join("crates/app"),
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\n",
        );
        let contribution = discover(fixture.path(), "crates/app", Vec::new());
        assert_eq!(
            contribution.target().runtimes[0].version,
            VersionRequirement::exact("nightly-2026-08-01").expect("exact nightly")
        );
        assert_eq!(
            contribution.target().runtimes[0].source.path,
            "rust-toolchain.toml"
        );
    }

    #[test]
    fn uses_workspace_rust_version_and_records_workspace_ownership() {
        let fixture = TempDir::new().expect("fixture");
        manifest(
            fixture.path(),
            "[workspace]\nmembers = [\"crates/app\"]\n[workspace.package]\nrust-version = \"1.78\"\n",
        );
        fs::create_dir_all(fixture.path().join("crates/app")).expect("package directory");
        manifest(
            &fixture.path().join("crates/app"),
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\nrust-version.workspace = true\n",
        );
        let contribution = discover(fixture.path(), "crates/app", vec![]);
        assert_eq!(contribution.target().workspace.as_deref(), Some("."));
        assert_eq!(contribution.target().package.as_deref(), Some("app"));
        assert_eq!(
            contribution.target().runtimes[0].version,
            VersionRequirement::minimum("1.78").expect("minimum")
        );
        assert_eq!(contribution.target().runtimes[0].source.path, "Cargo.toml");
    }

    #[test]
    fn malformed_or_missing_inherited_rust_version_fails_closed() {
        for workspace in [
            "[workspace]\nmembers = [\"crates/app\"]\n",
            "[workspace]\nmembers = [\"crates/app\"]\n[workspace.package]\nrust-version = 178\n",
        ] {
            let fixture = TempDir::new().expect("fixture");
            manifest(fixture.path(), workspace);
            fs::create_dir_all(fixture.path().join("crates/app")).expect("package directory");
            manifest(
                &fixture.path().join("crates/app"),
                "[package]\nname = \"app\"\nversion = \"0.1.0\"\nrust-version.workspace = true\n",
            );
            assert!(
                RustEnvironmentCapability
                    .discover(&request(fixture.path(), "crates/app", Vec::new()))
                    .is_err()
            );
        }
    }

    #[test]
    fn matching_legacy_selector_does_not_conflict_with_toml_components() {
        let fixture = TempDir::new().expect("fixture");
        manifest(
            fixture.path(),
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\n",
        );
        fs::write(
            fixture.path().join("rust-toolchain.toml"),
            "[toolchain]\nchannel = \"1.85.0\"\ncomponents = [\"rustfmt\"]\n",
        )
        .expect("toml");
        fs::write(fixture.path().join("rust-toolchain"), "1.85.0\n").expect("legacy");
        let contribution = discover(fixture.path(), ".", vec![]);
        assert!(contribution.conflicts().is_empty());
        assert_eq!(
            contribution.target().runtimes[0].components,
            vec![String::from("rustfmt")]
        );
    }

    #[test]
    fn toml_precedence_reports_a_typed_conflict_for_disagreement() {
        let fixture = TempDir::new().expect("fixture");
        manifest(
            fixture.path(),
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\n",
        );
        fs::write(
            fixture.path().join("rust-toolchain.toml"),
            "[toolchain]\nchannel = \"1.85.0\"\n",
        )
        .expect("toml");
        fs::write(fixture.path().join("rust-toolchain"), "1.84.0\n").expect("legacy");
        let contribution = discover(fixture.path(), ".", vec![]);
        assert_eq!(
            contribution.target().runtimes[0].version,
            VersionRequirement::exact("1.85.0").expect("version")
        );
        assert_eq!(
            contribution.conflicts()[0].code,
            "rust_toolchain_source_conflict"
        );
        assert_eq!(contribution.conflicts()[0].sources.len(), 2);
    }

    #[test]
    fn missing_runtime_source_is_unresolved_with_an_actionable_warning() {
        let fixture = TempDir::new().expect("fixture");
        manifest(
            fixture.path(),
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\n",
        );
        let contribution = discover(fixture.path(), ".", vec![]);
        assert!(matches!(
            contribution.target().runtimes[0].version,
            VersionRequirement::Unresolved { .. }
        ));
        assert_eq!(contribution.warnings()[0].code, "rust_runtime_unresolved");
        assert!(
            contribution.warnings()[0]
                .message
                .contains("rust-toolchain.toml")
        );
    }

    #[test]
    fn hashes_workspace_cargo_lock_with_a_relative_source() {
        let fixture = TempDir::new().expect("fixture");
        manifest(
            fixture.path(),
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\n",
        );
        fs::write(fixture.path().join("Cargo.lock"), "version = 3\n").expect("lock");
        let contribution = discover(fixture.path(), ".", vec![]);
        let lock = &contribution.target().dependency_locks[0];
        assert_eq!(lock.path, "Cargo.lock");
        assert_eq!(lock.owner_root, ".");
        assert_eq!(lock.source.path, "Cargo.lock");
        assert_eq!(
            lock.digest,
            "sha256:a6302849064e016e520e513a22aef99a2d874333e7fcbf0b2c2260cb6ffb42f6"
        );
    }

    #[cfg(unix)]
    #[test]
    fn source_file_symlink_escape_and_malformed_toolchain_fail_closed() {
        use std::os::unix::fs::symlink;

        let fixture = TempDir::new().expect("fixture");
        let repository = fixture.path().join("repository");
        fs::create_dir(&repository).expect("repository");
        let outside = fixture.path().join("outside.toml");
        fs::write(
            &outside,
            "[package]\nname = \"outside\"\nversion = \"0.1.0\"\n",
        )
        .expect("outside");
        symlink(&outside, repository.join("Cargo.toml")).expect("manifest link");
        assert!(
            RustEnvironmentCapability
                .discover(&request(&repository, ".", Vec::new()))
                .is_err()
        );

        fs::remove_file(repository.join("Cargo.toml")).expect("remove link");
        manifest(
            &repository,
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\n",
        );
        fs::write(repository.join("rust-toolchain.toml"), "[toolchain]\n")
            .expect("malformed toolchain");
        assert!(
            RustEnvironmentCapability
                .discover(&request(&repository, ".", Vec::new()))
                .is_err()
        );
    }

    #[test]
    fn selects_only_catalog_tools_for_enabled_signals() {
        let fixture = TempDir::new().expect("fixture");
        manifest(
            fixture.path(),
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\n",
        );
        let contribution = discover(
            fixture.path(),
            ".",
            vec![
                SignalKind::Coverage,
                SignalKind::Complexity,
                SignalKind::Mutation,
                SignalKind::Test,
                SignalKind::Deps,
                SignalKind::Size,
            ],
        );
        let tools = &contribution.target().signal_tools;
        assert_eq!(
            tools
                .iter()
                .map(|tool| tool.tool.as_str())
                .collect::<Vec<_>>(),
            ["cargo-llvm-cov", "cargo-mutants", "rust-code-analysis-cli"]
        );
        assert_eq!(
            tools
                .iter()
                .find(|tool| tool.tool == "cargo-llvm-cov")
                .expect("coverage tool")
                .version,
            VersionRequirement::exact("0.8.5").expect("version")
        );
        assert!(
            contribution.target().runtimes[0]
                .components
                .iter()
                .any(|component| component == "llvm-tools-preview")
        );
        assert!(
            tools
                .iter()
                .filter(|tool| matches!(
                    tool.tool.as_str(),
                    "cargo-mutants" | "rust-code-analysis-cli"
                ))
                .all(|tool| matches!(tool.version, VersionRequirement::Unresolved { .. }))
        );
    }
}
