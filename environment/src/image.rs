use crate::BackendError;
use crate::lock::{host_architecture, platform_architecture};
use crate::runtime::WORKSPACE;
use ayni_core::{
    EnvironmentLock, LockedRuntime, LockedSignalTool, LockedTargetEnvironment,
    ToolInstallationScope,
};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) const IMAGE_LOCK_LABEL: &str = "dev.ayni.environment.lock-fingerprint";
pub(crate) const IMAGE_BASE_LABEL: &str = "dev.ayni.environment.base-digest";
pub(crate) const IMAGE_SCHEMA_LABEL: &str = "dev.ayni.environment.schema";
pub(crate) const IMAGE_AYNI_LABEL: &str = "dev.ayni.environment.ayni-version";
pub(crate) const IMAGE_MISE_LABEL: &str = "dev.ayni.environment.mise-version";
pub(crate) const IMAGE_PLATFORM_LABEL: &str = "dev.ayni.environment.platform";
pub(crate) const IMAGE_SCHEMA_VERSION: &str = "0.2.0";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImagePlan {
    pub tag: String,
    pub dockerfile: String,
    pub mise_toml: String,
    pub platform: String,
}

#[derive(Default)]
struct ProvisioningInventory {
    tools: BTreeMap<String, BTreeSet<String>>,
    rust_components: BTreeSet<String>,
    rust_targets: BTreeSet<String>,
}

/// Construct a deterministic repository-image plan using only a validated
/// lock. Project-scoped tools remain native dependencies and are deliberately
/// not translated into mise providers by this generic backend.
pub fn image_plan(lock: &EnvironmentLock) -> Result<ImagePlan, BackendError> {
    let architecture = host_architecture()?;
    let platform = format!("linux/{}", platform_architecture(architecture));
    let inventory = provisioning_inventory(lock)?;
    Ok(ImagePlan {
        tag: image_tag(lock, architecture),
        dockerfile: dockerfile(lock, &platform, &inventory),
        mise_toml: mise_toml(inventory.tools),
        platform,
    })
}

fn image_tag(lock: &EnvironmentLock, architecture: ayni_core::Architecture) -> String {
    let fingerprint = lock
        .fingerprint()
        .strip_prefix("sha256:")
        .unwrap_or(lock.fingerprint());
    format!(
        "ayni-env:lock-{}-linux-{}",
        &fingerprint[..16.min(fingerprint.len())],
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
            &runtime.components,
            &mut inventory.rust_components,
        )?;
        add_rust_items("target", &runtime.targets, &mut inventory.rust_targets)?;
    }
    add_tool(&mut inventory.tools, &runtime.runtime, &runtime.version);
    Ok(())
}

fn add_rust_items(
    kind: &str,
    values: &[String],
    destination: &mut BTreeSet<String>,
) -> Result<(), BackendError> {
    for value in values {
        validate_rustup_item(kind, value)?;
        destination.insert(value.clone());
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

fn dockerfile(lock: &EnvironmentLock, platform: &str, inventory: &ProvisioningInventory) -> String {
    let provisioning_env = rust_provisioning_env(inventory);
    let base = lock.provisioning_base();
    format!(
        "FROM {}@{}\nUSER ayni\nCOPY --chown=10001:10001 mise.toml /etc/ayni/mise.toml\nRUN chmod 0444 /etc/ayni/mise.toml\nENV MISE_CONFIG_FILE=/etc/ayni/mise.toml MISE_TRUSTED_CONFIG_PATHS=/etc/ayni\n{provisioning_env}RUN mise trust /etc/ayni/mise.toml && mise install --yes && mise reshim\nENV MISE_AUTO_INSTALL=0 MISE_CONFIG_FILE=/etc/ayni/mise.toml\nLABEL {IMAGE_SCHEMA_LABEL}=\"{IMAGE_SCHEMA_VERSION}\" {IMAGE_LOCK_LABEL}=\"{}\" {IMAGE_BASE_LABEL}=\"{}\" {IMAGE_AYNI_LABEL}=\"{}\" {IMAGE_MISE_LABEL}=\"{}\" {IMAGE_PLATFORM_LABEL}=\"{}\"\nWORKDIR {WORKSPACE}\n",
        base.reference,
        base.digest,
        lock.fingerprint(),
        base.digest,
        lock.ayni_version(),
        base.mise_version,
        platform,
    )
}

fn rust_provisioning_env(inventory: &ProvisioningInventory) -> String {
    let mut output = String::new();
    push_rust_provisioning_env(
        &mut output,
        "MISE_RUSTUP_COMPONENTS",
        &inventory.rust_components,
    );
    push_rust_provisioning_env(&mut output, "MISE_RUSTUP_TARGETS", &inventory.rust_targets);
    output
}

fn push_rust_provisioning_env(output: &mut String, name: &str, values: &BTreeSet<String>) {
    if values.is_empty() {
        return;
    }
    let value = values.iter().cloned().collect::<Vec<_>>().join(",");
    output.push_str(&format!(
        "ENV {name}={}\n",
        serde_json::to_string(&value).expect("string serialization")
    ));
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
