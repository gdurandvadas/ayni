use crate::BackendError;
use crate::lock::{host_architecture, platform_architecture};
use crate::runtime::WORKSPACE;
use ayni_core::{
    DependencyPreparationPlan, EnvironmentLock, LockedRuntime, LockedSignalTool,
    LockedTargetEnvironment, ToolInstallationScope,
};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) const IMAGE_LOCK_LABEL: &str = "dev.ayni.environment.lock-fingerprint";
pub(crate) const IMAGE_BASE_LABEL: &str = "dev.ayni.environment.base-digest";
pub(crate) const IMAGE_SCHEMA_LABEL: &str = "dev.ayni.environment.schema";
pub(crate) const IMAGE_AYNI_LABEL: &str = "dev.ayni.environment.ayni-version";
pub(crate) const IMAGE_MISE_LABEL: &str = "dev.ayni.environment.mise-version";
pub(crate) const IMAGE_PLATFORM_LABEL: &str = "dev.ayni.environment.platform";
pub(crate) const IMAGE_PREPARATION_LABEL: &str = "dev.ayni.environment.preparation-digest";
pub(crate) const IMAGE_SCHEMA_VERSION: &str = "0.3.0";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImagePlan {
    pub tag: String,
    pub dockerfile: String,
    pub mise_toml: String,
    pub platform: String,
    pub preparation_digest: String,
}

#[derive(Default)]
struct ProvisioningInventory {
    tools: BTreeMap<String, BTreeSet<String>>,
    rust_components: BTreeMap<String, BTreeSet<String>>,
    rust_targets: BTreeMap<String, BTreeSet<String>>,
}

/// Construct a deterministic repository-image plan using only a validated
/// lock. Project-scoped tools remain native dependencies and are deliberately
/// not translated into mise providers by this generic backend.
pub fn image_plan(lock: &EnvironmentLock) -> Result<ImagePlan, BackendError> {
    image_plan_with_preparation(lock, &[])
}

pub fn image_plan_with_preparation(
    lock: &EnvironmentLock,
    preparations: &[DependencyPreparationPlan],
) -> Result<ImagePlan, BackendError> {
    let architecture = host_architecture()?;
    let platform = format!("linux/{}", platform_architecture(architecture));
    let inventory = provisioning_inventory(lock)?;
    let preparation_digest = crate::preparation::preparation_digest(preparations)?;
    Ok(ImagePlan {
        tag: image_tag(lock, &preparation_digest, architecture),
        dockerfile: dockerfile(
            lock,
            &platform,
            &inventory,
            preparations,
            &preparation_digest,
        )?,
        mise_toml: mise_toml(inventory.tools),
        platform,
        preparation_digest,
    })
}

fn image_tag(
    lock: &EnvironmentLock,
    preparation_digest: &str,
    architecture: ayni_core::Architecture,
) -> String {
    let fingerprint = lock
        .fingerprint()
        .strip_prefix("sha256:")
        .unwrap_or(lock.fingerprint());
    let preparation = preparation_digest
        .strip_prefix("sha256:")
        .unwrap_or(preparation_digest);
    format!(
        "ayni-env:lock-{}-prep-{}-linux-{}",
        &fingerprint[..16.min(fingerprint.len())],
        &preparation[..16.min(preparation.len())],
        platform_architecture(architecture)
    )
}

fn provisioning_inventory(lock: &EnvironmentLock) -> Result<ProvisioningInventory, BackendError> {
    let mut inventory = ProvisioningInventory::default();
    for target in lock.targets() {
        add_target(target, &mut inventory)?;
    }
    Ok(inventory)
}

fn add_target(
    target: &LockedTargetEnvironment,
    inventory: &mut ProvisioningInventory,
) -> Result<(), BackendError> {
    for runtime in &target.runtimes {
        add_runtime(runtime, inventory)?;
    }
    if let Some(manager) = &target.package_manager {
        add_tool(&mut inventory.tools, &manager.family, &manager.version);
    }
    add_signal_tools(&target.signal_tools, &mut inventory.tools)
}

fn add_runtime(
    runtime: &LockedRuntime,
    inventory: &mut ProvisioningInventory,
) -> Result<(), BackendError> {
    if runtime.runtime == "rust" {
        add_rust_items(
            "component",
            &runtime.version,
            &runtime.components,
            &mut inventory.rust_components,
        )?;
        add_rust_items(
            "target",
            &runtime.version,
            &runtime.targets,
            &mut inventory.rust_targets,
        )?;
    }
    add_tool(&mut inventory.tools, &runtime.runtime, &runtime.version);
    Ok(())
}

fn add_rust_items(
    kind: &str,
    version: &str,
    values: &[String],
    destination: &mut BTreeMap<String, BTreeSet<String>>,
) -> Result<(), BackendError> {
    for value in values {
        validate_rustup_item(kind, value)?;
        destination
            .entry(version.to_owned())
            .or_default()
            .insert(value.clone());
    }
    Ok(())
}

fn add_signal_tools(
    signal_tools: &[LockedSignalTool],
    tools: &mut BTreeMap<String, BTreeSet<String>>,
) -> Result<(), BackendError> {
    for tool in signal_tools {
        match tool.scope {
            ToolInstallationScope::Project => continue,
            ToolInstallationScope::Runtime => add_tool(tools, &tool.tool, &tool.version),
            ToolInstallationScope::Isolated if tool.provider == "cargo-install" => {
                add_tool(tools, &format!("cargo:{}", tool.tool), &tool.version);
            }
            ToolInstallationScope::Isolated
                if tool.provider.starts_with("go:") || tool.provider.starts_with("pipx:") =>
            {
                add_tool(tools, &tool.provider, &tool.version);
            }
            ToolInstallationScope::Isolated => {
                return Err(BackendError::environment(format!(
                    "environment backend does not support isolated provider {} for {}",
                    tool.provider, tool.tool
                )));
            }
        }
    }
    Ok(())
}

fn add_tool(tools: &mut BTreeMap<String, BTreeSet<String>>, tool: &str, version: &str) {
    tools
        .entry(tool.to_owned())
        .or_default()
        .insert(version.to_owned());
}

fn mise_toml(tools: BTreeMap<String, BTreeSet<String>>) -> String {
    let mut output = String::from("[tools]\n");
    for (tool, versions) in tools {
        let values = versions
            .into_iter()
            .map(|version| serde_json::to_string(&version).expect("string serialization"))
            .collect::<Vec<_>>();
        let key = serde_json::to_string(&tool).expect("string serialization");
        if values.len() == 1 {
            output.push_str(&format!("{key} = {}\n", values[0]));
        } else {
            output.push_str(&format!("{key} = [{}]\n", values.join(", ")));
        }
    }
    output
}

fn dockerfile(
    lock: &EnvironmentLock,
    platform: &str,
    inventory: &ProvisioningInventory,
    preparations: &[DependencyPreparationPlan],
    preparation_digest: &str,
) -> Result<String, BackendError> {
    let rustup_provisioning = rustup_provisioning(inventory);
    let base = lock.provisioning_base();
    let preparation = crate::preparation::dockerfile_fragment(lock, preparations)?;
    Ok(format!(
        "FROM {}@{} AS ayni-runtime\nUSER ayni\nCOPY --chown=10001:10001 mise.toml /etc/ayni/mise.toml\nRUN chmod 0444 /etc/ayni/mise.toml\nENV MISE_CONFIG_FILE=/etc/ayni/mise.toml MISE_TRUSTED_CONFIG_PATHS=/etc/ayni\nRUN mise trust /etc/ayni/mise.toml && mise install --yes && mise reshim\n{rustup_provisioning}ENV MISE_AUTO_INSTALL=0 MISE_CONFIG_FILE=/etc/ayni/mise.toml\n{preparation}LABEL {IMAGE_SCHEMA_LABEL}=\"{IMAGE_SCHEMA_VERSION}\" {IMAGE_LOCK_LABEL}=\"{}\" {IMAGE_BASE_LABEL}=\"{}\" {IMAGE_AYNI_LABEL}=\"{}\" {IMAGE_MISE_LABEL}=\"{}\" {IMAGE_PLATFORM_LABEL}=\"{}\" {IMAGE_PREPARATION_LABEL}=\"{}\"\nWORKDIR {WORKSPACE}\n",
        base.reference,
        base.digest,
        lock.fingerprint(),
        base.digest,
        lock.ayni_version(),
        base.mise_version,
        platform,
        preparation_digest,
    ))
}

fn rustup_provisioning(inventory: &ProvisioningInventory) -> String {
    let mut output = String::new();
    push_rustup_commands(&mut output, "component", &inventory.rust_components);
    push_rustup_commands(&mut output, "target", &inventory.rust_targets);
    output
}

fn push_rustup_commands(
    output: &mut String,
    kind: &str,
    versions: &BTreeMap<String, BTreeSet<String>>,
) {
    for (version, values) in versions {
        let mut command = vec![
            String::from("rustup"),
            kind.to_owned(),
            String::from("add"),
            String::from("--toolchain"),
            version.clone(),
        ];
        command.extend(values.iter().cloned());
        output.push_str("RUN ");
        output.push_str(&serde_json::to_string(&command).expect("rustup argv serialization"));
        output.push('\n');
    }
}

fn validate_rustup_item(kind: &str, value: &str) -> Result<(), BackendError> {
    let safe = !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'));
    if safe {
        Ok(())
    } else {
        Err(BackendError::environment(format!(
            "locked Rust {kind} cannot be provisioned safely: {value}"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ayni_core::{
        LockedRequirementSource, RequirementConfidence, SignalKind, ToolInstallationScope,
    };

    fn source() -> LockedRequirementSource {
        LockedRequirementSource {
            kind: "test".into(),
            path: "go.mod".into(),
            digest: None,
            confidence: RequirementConfidence::Exact,
        }
    }

    #[test]
    fn isolated_mise_provider_coordinates_are_preserved_without_language_tool_logic() {
        let tools = vec![LockedSignalTool {
            tool: "gocyclo".into(),
            version: "0.6.0".into(),
            provider: "go:github.com/fzipp/gocyclo/cmd/gocyclo".into(),
            scope: ToolInstallationScope::Isolated,
            signals: vec![SignalKind::Complexity],
            source: source(),
        }];
        let mut inventory = BTreeMap::new();
        add_signal_tools(&tools, &mut inventory).expect("provider");
        assert_eq!(
            mise_toml(inventory),
            "[tools]\n\"go:github.com/fzipp/gocyclo/cmd/gocyclo\" = \"0.6.0\"\n"
        );
    }

    #[test]
    fn unknown_isolated_provider_fails_closed() {
        let tools = vec![LockedSignalTool {
            tool: "unknown".into(),
            version: "1.0.0".into(),
            provider: "unknown-provider".into(),
            scope: ToolInstallationScope::Isolated,
            signals: Vec::new(),
            source: source(),
        }];
        assert!(add_signal_tools(&tools, &mut BTreeMap::new()).is_err());
    }
}
