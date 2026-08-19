use crate::application::{EnvShowOperation, OutputFormat};
use ayni_adapters_common::environment::environment_discovery_request;
use ayni_core::{
    AdapterRegistry, Architecture, AyniPolicy, DebianPackageRequirement, DockerAccess,
    EnvironmentConflict, EnvironmentPlan, Libc, MiseToolRequirement, OperatingSystem,
    RepositoryIdentity, RequirementConfidence, RequirementSource, TargetIdentity, TargetPlatform,
    VersionRequirement,
};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fmt::Write;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

pub(crate) type ShowError = crate::application_error::ApplicationError;

pub(crate) fn show(operation: EnvShowOperation, registry: &AdapterRegistry) -> ExitCode {
    match build_plan(&operation, registry) {
        Ok(plan) => {
            match operation.output {
                OutputFormat::Json => match serde_json::to_string_pretty(&plan) {
                    Ok(output) => println!("{output}"),
                    Err(error) => {
                        eprintln!("failed to render environment plan: {error}");
                        return ExitCode::from(4);
                    }
                },
                OutputFormat::Human => print_human(&plan),
                OutputFormat::Markdown => unreachable!("env show does not accept markdown output"),
            }
            ExitCode::SUCCESS
        }
        Err(error) => crate::application_error::render_error(error),
    }
}

pub(crate) fn build_plan(
    operation: &EnvShowOperation,
    registry: &AdapterRegistry,
) -> Result<EnvironmentPlan, ShowError> {
    let (repo_root, config, config_bytes, policy) = load_context(operation)?;
    let platforms = default_platforms();
    let (targets, warnings, mut conflicts) =
        discover_targets(&repo_root, &policy, &platforms, registry)?;
    let tools = repository_tools(&repo_root, &config, &policy)?;
    let debian_packages = repository_debian_packages(&repo_root, &config, &policy)?;
    conflicts.extend(generic_tool_conflicts(&tools, &targets)?);
    EnvironmentPlan::new(
        repository_identity(&repo_root, &config_bytes)?,
        platforms,
        targets,
        warnings,
        conflicts,
    )
    .and_then(|plan| plan.with_tools(tools))
    .and_then(|plan| plan.with_debian_packages(debian_packages))
    .and_then(|plan| plan.with_capabilities(policy.environment_capabilities()))
    .map_err(|error| {
        ShowError::environment(format!(
            "failed to aggregate environment plan from {}: {error}",
            config.display()
        ))
    })
}

fn load_context(
    operation: &EnvShowOperation,
) -> Result<(PathBuf, PathBuf, Vec<u8>, AyniPolicy), ShowError> {
    let repo_root = operation.repo_root.canonicalize().map_err(|error| {
        ShowError::input(format!(
            "failed to establish repository root {}: {error}",
            operation.repo_root.display()
        ))
    })?;
    if !repo_root.is_dir() {
        return Err(ShowError::input(format!(
            "repository root is not a directory: {}",
            repo_root.display()
        )));
    }
    let config = resolve_config_path(&repo_root, &operation.config)?;
    let config_bytes = fs::read(&config).map_err(|error| {
        ShowError::input(format!("failed to read {}: {error}", config.display()))
    })?;
    let config_content = std::str::from_utf8(&config_bytes).map_err(|error| {
        ShowError::input(format!(
            "failed to parse {} as UTF-8: {error}",
            config.display()
        ))
    })?;
    let policy = AyniPolicy::parse(config_content).map_err(|error| {
        ShowError::input(format!(
            "failed to load environment configuration {}: {error}",
            config.display()
        ))
    })?;
    Ok((repo_root, config, config_bytes, policy))
}

fn repository_tools(
    repo_root: &Path,
    config: &Path,
    policy: &AyniPolicy,
) -> Result<Vec<MiseToolRequirement>, ShowError> {
    let path = config
        .strip_prefix(repo_root)
        .map_err(|_| ShowError::input("environment contract escapes repository root"))?
        .to_string_lossy()
        .replace('\\', "/");
    policy
        .environment_tools()
        .iter()
        .map(|(tool, version)| {
            Ok(MiseToolRequirement {
                tool: tool.clone(),
                version: VersionRequirement::exact(version).map_err(|error| {
                    ShowError::input(format!(
                        "environment.tools.{tool} must be an exact version: {error}"
                    ))
                })?,
                source: RequirementSource::new(
                    "environment_tool",
                    &path,
                    Some(format!("environment.tools.{tool}")),
                    RequirementConfidence::Declared,
                )
                .map_err(|error| ShowError::input(error.to_string()))?,
            })
        })
        .collect()
}

fn repository_debian_packages(
    repo_root: &Path,
    config: &Path,
    policy: &AyniPolicy,
) -> Result<Vec<DebianPackageRequirement>, ShowError> {
    let path = config
        .strip_prefix(repo_root)
        .map_err(|_| ShowError::input("environment contract escapes repository root"))?
        .to_string_lossy()
        .replace('\\', "/");
    let configured = policy.environment_debian_packages();
    let requested =
        ayni_environment::resolve_debian_packages(configured, policy.environment_capabilities());
    requested
        .into_iter()
        .map(|package| {
            let declared = configured.contains(&package);
            Ok(DebianPackageRequirement {
                package: package.clone(),
                source: RequirementSource::new(
                    if declared {
                        "environment_debian_package"
                    } else {
                        "environment_capability"
                    },
                    &path,
                    Some(if declared {
                        format!("environment.debian.packages:{package}")
                    } else {
                        format!("environment.capabilities:{package}")
                    }),
                    RequirementConfidence::Declared,
                )
                .map_err(|error| ShowError::input(error.to_string()))?,
            })
        })
        .collect()
}

fn generic_tool_conflicts(
    tools: &[MiseToolRequirement],
    targets: &[ayni_core::TargetEnvironment],
) -> Result<Vec<EnvironmentConflict>, ShowError> {
    let adapter_tools = adapter_mise_tools(targets)?;
    let mut conflicts = Vec::new();
    for tool in tools {
        for adapter in adapter_tools
            .iter()
            .filter(|adapter| adapter.coordinate == tool.tool && adapter.version != &tool.version)
        {
            conflicts.push(EnvironmentConflict {
                code: String::from("environment_tool_version_conflict"),
                message: format!(
                    "repository tool {} {:?} conflicts with adapter requirement {:?}",
                    tool.tool, tool.version, adapter.version
                ),
                target: Some(adapter.identity.clone()),
                sources: vec![tool.source.clone(), adapter.source.clone()],
            });
        }
    }
    Ok(conflicts)
}

struct AdapterMiseTool<'a> {
    identity: &'a TargetIdentity,
    coordinate: String,
    version: &'a VersionRequirement,
    source: &'a RequirementSource,
}

fn adapter_mise_tools(
    targets: &[ayni_core::TargetEnvironment],
) -> Result<Vec<AdapterMiseTool<'_>>, ShowError> {
    let mut tools = Vec::new();
    for target in targets {
        tools.extend(target.runtimes.iter().map(|runtime| AdapterMiseTool {
            identity: &target.target,
            coordinate: runtime.runtime.clone(),
            version: &runtime.version,
            source: &runtime.source,
        }));
        tools.extend(
            target
                .package_manager
                .iter()
                .map(|manager| AdapterMiseTool {
                    identity: &target.target,
                    coordinate: manager.family.clone(),
                    version: &manager.version,
                    source: &manager.source,
                }),
        );
        for tool in &target.signal_tools {
            if let Some(coordinate) =
                ayni_environment::signal_tool_coordinate(tool.scope, &tool.tool, &tool.provider)
                    .map_err(|error| ShowError::environment(error.message))?
            {
                tools.push(AdapterMiseTool {
                    identity: &target.target,
                    coordinate,
                    version: &tool.version,
                    source: &tool.source,
                });
            }
        }
    }
    Ok(tools)
}

type DiscoveryParts = (
    Vec<ayni_core::TargetEnvironment>,
    Vec<ayni_core::EnvironmentWarning>,
    Vec<ayni_core::EnvironmentConflict>,
);

fn discover_targets(
    repo_root: &Path,
    policy: &AyniPolicy,
    platforms: &[TargetPlatform],
    registry: &AdapterRegistry,
) -> Result<DiscoveryParts, ShowError> {
    let enabled_signals = policy.enabled_signals();
    let mut seen = BTreeSet::new();
    let mut targets = Vec::new();
    let mut warnings = Vec::new();
    let mut conflicts = Vec::new();
    for language in policy
        .enabled_languages()
        .map_err(|error| ShowError::input(format!("invalid environment configuration: {error}")))?
    {
        let adapter = registry
            .adapters()
            .iter()
            .find(|adapter| adapter.language() == language)
            .ok_or_else(|| {
                ShowError::environment(format!(
                    "{language} adapter is not registered for environment discovery"
                ))
            })?;
        for root in policy.roots_for(language) {
            let identity = TargetIdentity::new(language, root).map_err(|error| {
                ShowError::input(format!(
                    "invalid environment target {language}:{root}: {error}"
                ))
            })?;
            if !seen.insert(identity.clone()) {
                continue;
            }
            ensure_detected(repo_root, root, adapter.as_ref())?;
            let request = environment_discovery_request(
                repo_root.to_path_buf(),
                identity,
                enabled_signals.iter().copied(),
                platforms.to_vec(),
            )
            .map_err(|error| ShowError::environment(error.to_string()))?;
            let contribution = adapter
                .discover_environment(&request)
                .map_err(|error| ShowError::environment(error.to_string()))?;
            let (target, contribution_warnings, contribution_conflicts) = contribution.into_parts();
            targets.push(target);
            warnings.extend(contribution_warnings);
            conflicts.extend(contribution_conflicts);
        }
    }
    Ok((targets, warnings, conflicts))
}

fn ensure_detected(
    repo_root: &Path,
    root: &str,
    adapter: &dyn ayni_core::LanguageAdapter,
) -> Result<(), ShowError> {
    let target_root = if root == "." {
        repo_root.to_path_buf()
    } else {
        repo_root.join(root)
    };
    let detection = adapter.detect(&target_root);
    if detection.detected {
        return Ok(());
    }
    Err(ShowError::environment(detection.reason.unwrap_or_else(
        || {
            format!(
                "configured {} target {root} was not detected at {}",
                adapter.language(),
                target_root.display()
            )
        },
    )))
}

fn repository_identity(
    repo_root: &Path,
    config_bytes: &[u8],
) -> Result<RepositoryIdentity, ShowError> {
    let name = repo_root
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .ok_or_else(|| {
            ShowError::input(format!(
                "repository root has no usable final component: {}",
                repo_root.display()
            ))
        })?
        .to_owned();
    Ok(RepositoryIdentity {
        name,
        contract_digest: format!("{:x}", Sha256::digest(config_bytes)),
    })
}

fn resolve_config_path(repo_root: &Path, configured: &Path) -> Result<PathBuf, ShowError> {
    let candidate = if configured.is_absolute() {
        configured.to_path_buf()
    } else {
        repo_root.join(configured)
    };
    let config = candidate.canonicalize().map_err(|error| {
        ShowError::input(format!(
            "failed to resolve config {}: {error}",
            candidate.display()
        ))
    })?;
    if !config.starts_with(repo_root) {
        return Err(ShowError::input(format!(
            "config {} escapes repository root {}",
            config.display(),
            repo_root.display()
        )));
    }
    if !config.is_file() {
        return Err(ShowError::input(format!(
            "config is not a file: {}",
            config.display()
        )));
    }
    Ok(config)
}

pub(crate) fn default_platforms() -> Vec<TargetPlatform> {
    vec![
        TargetPlatform {
            os: OperatingSystem::Linux,
            architecture: Architecture::Amd64,
            libc: Libc::Glibc,
        },
        TargetPlatform {
            os: OperatingSystem::Linux,
            architecture: Architecture::Arm64,
            libc: Libc::Glibc,
        },
    ]
}

fn print_human(plan: &EnvironmentPlan) {
    let mut output = String::new();
    writeln!(output, "environment plan {}", plan.repository().name).expect("string write");
    writeln!(
        output,
        "contract digest: {}",
        plan.repository().contract_digest
    )
    .expect("string write");
    render_platforms(&mut output, plan);
    render_repository_tools(&mut output, plan);
    render_debian_packages(&mut output, plan);
    render_capabilities(&mut output, plan);
    render_targets(&mut output, plan);
    render_diagnostics(&mut output, plan);
    print!("{output}");
}

fn render_platforms(output: &mut String, plan: &EnvironmentPlan) {
    writeln!(output, "platforms:").expect("string write");
    for platform in plan.platforms() {
        writeln!(
            output,
            "  - {:?}/{:?}/{:?}",
            platform.os, platform.architecture, platform.libc
        )
        .expect("string write");
    }
}

fn render_repository_tools(output: &mut String, plan: &EnvironmentPlan) {
    if plan.tools().is_empty() {
        return;
    }
    writeln!(output, "repository tools:").expect("string write");
    for tool in plan.tools() {
        writeln!(
            output,
            "  - {} {:?} [{} {} {:?}]",
            tool.tool, tool.version, tool.source.kind, tool.source.path, tool.source.confidence
        )
        .expect("string write");
    }
}

fn render_debian_packages(output: &mut String, plan: &EnvironmentPlan) {
    if plan.debian_packages().is_empty() {
        return;
    }
    writeln!(output, "Debian packages:").expect("string write");
    for package in plan.debian_packages() {
        writeln!(output, "  - {}", package.package).expect("string write");
    }
}

fn render_capabilities(output: &mut String, plan: &EnvironmentPlan) {
    let capabilities = plan.capabilities();
    writeln!(
        output,
        "runtime capabilities: docker={:?} network={:?}",
        capabilities.docker, capabilities.network
    )
    .expect("string write");
    if capabilities.docker == DockerAccess::Socket {
        writeln!(
            output,
            "  warning: Docker socket access grants the environment control over the host Docker daemon"
        )
        .expect("string write");
    }
}

fn render_targets(output: &mut String, plan: &EnvironmentPlan) {
    writeln!(output, "targets:").expect("string write");
    for target in plan.targets() {
        writeln!(
            output,
            "  - {}:{}",
            target.target.language, target.target.root
        )
        .expect("string write");
        render_target_context(output, target);
        render_target_requirements(output, target);
    }
}

fn render_target_context(output: &mut String, target: &ayni_core::TargetEnvironment) {
    if let Some(workspace) = &target.workspace {
        writeln!(output, "    workspace: {workspace}").expect("string write");
    }
    if let Some(package) = &target.package {
        writeln!(output, "    package: {package}").expect("string write");
    }
}

fn render_target_requirements(output: &mut String, target: &ayni_core::TargetEnvironment) {
    for runtime in &target.runtimes {
        writeln!(
            output,
            "    runtime: {} {:?} [{} {} {:?}]",
            runtime.runtime,
            runtime.version,
            runtime.source.kind,
            runtime.source.path,
            runtime.source.confidence
        )
        .expect("string write");
        if !runtime.components.is_empty() {
            writeln!(
                output,
                "      components: {}",
                runtime.components.join(", ")
            )
            .expect("string write");
        }
        if !runtime.targets.is_empty() {
            writeln!(output, "      targets: {}", runtime.targets.join(", "))
                .expect("string write");
        }
    }
    render_package_manager(output, target.package_manager.as_ref());
    for tool in &target.signal_tools {
        writeln!(
            output,
            "    tool: {} {:?} provider={} scope={:?} provisioning={:?} modifies_checkout={} signals={:?} [{} {} {:?}]",
            tool.tool,
            tool.version,
            tool.provider,
            tool.scope,
            tool.provisioning,
            tool.modifies_checkout,
            tool.signals,
            tool.source.kind,
            tool.source.path,
            tool.source.confidence
        )
        .expect("string write");
    }
    for requirement in &target.system_requirements {
        writeln!(
            output,
            "    system: {:?} {} provisioning={:?} [{} {} {:?}]",
            requirement.kind,
            requirement.name,
            requirement.provisioning,
            requirement.source.kind,
            requirement.source.path,
            requirement.source.confidence
        )
        .expect("string write");
    }
    for lock in &target.dependency_locks {
        writeln!(
            output,
            "    dependency lock: {} {} owner={} [{} {} {:?}]",
            lock.path,
            lock.digest,
            lock.owner_root,
            lock.source.kind,
            lock.source.path,
            lock.source.confidence
        )
        .expect("string write");
    }
}

fn render_package_manager(
    output: &mut String,
    manager: Option<&ayni_core::PackageManagerRequirement>,
) {
    let Some(manager) = manager else {
        return;
    };
    writeln!(
        output,
        "    package manager: {} {:?} owner={} [{} {} {:?}]",
        manager.family,
        manager.version,
        manager.ownership_root,
        manager.source.kind,
        manager.source.path,
        manager.source.confidence
    )
    .expect("string write");
}

fn render_diagnostics(output: &mut String, plan: &EnvironmentPlan) {
    if !plan.warnings().is_empty() {
        writeln!(output, "warnings:").expect("string write");
        for warning in plan.warnings() {
            writeln!(output, "  - {}: {}", warning.code, warning.message).expect("string write");
        }
    }
    if !plan.conflicts().is_empty() {
        writeln!(output, "conflicts:").expect("string write");
        for conflict in plan.conflicts() {
            writeln!(output, "  - {}: {}", conflict.code, conflict.message).expect("string write");
            for source in &conflict.sources {
                let detail = source
                    .detail
                    .as_deref()
                    .map(|detail| format!(" ({detail})"))
                    .unwrap_or_default();
                writeln!(
                    output,
                    "      source: {} {} {:?}{detail}",
                    source.kind, source.path, source.confidence
                )
                .expect("string write");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ayni_adapters_node::NodeAdapter;
    use ayni_adapters_rust::RustAdapter;
    use std::sync::Arc;
    use tempfile::TempDir;

    fn registry() -> AdapterRegistry {
        let mut registry = AdapterRegistry::new();
        registry.register(Arc::new(RustAdapter::new()));
        registry.register(Arc::new(NodeAdapter::new()));
        registry
    }

    #[test]
    fn aggregates_mixed_targets_deterministically_without_writes() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join(".ayni.toml"), "[checks]\ntest = true\n[languages]\nenabled = [\"node\", \"rust\", \"node\"]\n[rust]\nroots = [\"rust\"]\n[node]\nroots = [\"node\"]\n[environment.tools]\nprotoc = \"35.1\"\n[environment.debian]\npackages = [\"libssl-dev\"]\n[environment.docker]\naccess = \"socket\"\nnetwork = \"bridge\"\n").unwrap();
        fs::create_dir(temp.path().join("rust")).unwrap();
        fs::write(
            temp.path().join("rust/Cargo.toml"),
            "[package]\nname = \"x\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .unwrap();
        fs::create_dir(temp.path().join("node")).unwrap();
        fs::write(
            temp.path().join("node/package.json"),
            "{\"name\":\"x\",\"engines\":{\"node\":\">=20\"}} ",
        )
        .unwrap();
        let operation = EnvShowOperation {
            config: PathBuf::from(".ayni.toml"),
            repo_root: temp.path().to_path_buf(),
            output: OutputFormat::Json,
        };
        let one = build_plan(&operation, &registry()).unwrap();
        let two = build_plan(&operation, &registry()).unwrap();
        assert_eq!(
            serde_json::to_vec(&one).unwrap(),
            serde_json::to_vec(&two).unwrap()
        );
        assert_eq!(one.targets().len(), 2);
        assert_eq!(one.tools()[0].tool, "protoc");
        assert_eq!(
            one.debian_packages()
                .iter()
                .map(|package| package.package.as_str())
                .collect::<Vec<_>>(),
            ["docker.io", "libssl-dev"]
        );
        assert_eq!(one.capabilities().docker, DockerAccess::Socket);
        assert_eq!(one.capabilities().network, ayni_core::NetworkAccess::Bridge);
        assert!(!temp.path().join(".ayni").exists());
    }

    #[test]
    fn repository_tool_conflicts_include_adapter_provider_coordinates() {
        let temp = TempDir::new().unwrap();
        fs::write(
            temp.path().join(".ayni.toml"),
            "[checks]\ntest=false\ncoverage=true\nsize=false\ncomplexity=false\ndeps=false\nmutation=false\n[languages]\nenabled=[\"rust\"]\n[environment.tools]\n\"cargo:cargo-llvm-cov\"=\"0.1.0\"\n",
        )
        .unwrap();
        fs::write(
            temp.path().join("Cargo.toml"),
            "[package]\nname=\"fixture\"\nversion=\"0.1.0\"\nrust-version=\"1.97.1\"\n",
        )
        .unwrap();
        fs::write(temp.path().join("Cargo.lock"), "version = 4\n").unwrap();
        let operation = EnvShowOperation {
            config: PathBuf::from(".ayni.toml"),
            repo_root: temp.path().to_path_buf(),
            output: OutputFormat::Json,
        };
        let plan = build_plan(&operation, &registry()).expect("plan with conflict");
        assert!(
            plan.conflicts()
                .iter()
                .any(|conflict| conflict.code == "environment_tool_version_conflict")
        );
    }

    #[test]
    fn rejects_config_escape() {
        let temp = TempDir::new().unwrap();
        let outside = TempDir::new().unwrap();
        fs::write(
            outside.path().join("policy.toml"),
            "[languages]\nenabled = [\"rust\"]",
        )
        .unwrap();
        let operation = EnvShowOperation {
            config: outside.path().join("policy.toml"),
            repo_root: temp.path().to_path_buf(),
            output: OutputFormat::Human,
        };
        assert!(
            build_plan(&operation, &registry())
                .unwrap_err()
                .message
                .contains("escapes repository root")
        );
    }
}
