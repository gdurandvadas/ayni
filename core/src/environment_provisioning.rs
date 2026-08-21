use crate::environment::{
    EnvironmentPlanError, RequirementSource, VersionRequirement, normalize_source,
    normalize_version_requirement, required_label,
};
use serde::{Deserialize, Deserializer, Serialize};

/// Repository-wide Mise tool declared independently of a quality-language
/// adapter. This keeps generic environment provisioning open-ended while
/// `Language` remains the closed set of adapters that provide quality signals.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct MiseToolRequirement {
    pub tool: String,
    pub version: VersionRequirement,
    pub source: RequirementSource,
}

/// Repository-wide package installed from the Debian repositories configured
/// by the immutable provisioning base.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DebianPackageRequirement {
    pub package: String,
    pub source: RequirementSource,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DockerAccess {
    #[default]
    None,
    Socket,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkAccess {
    #[default]
    None,
    Bridge,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct EnvironmentCapabilities {
    pub docker: DockerAccess,
    pub network: NetworkAccess,
}

/// Resource ceilings applied to every managed runtime container.
///
/// Values use deterministic integer units so policy, plan, and lock retain one
/// representation. Setting `memory_swap_mib` equal to `memory_mib` disables
/// additional swap under Docker and Podman semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct EnvironmentResourceLimits {
    pub cpus: u16,
    pub memory_mib: u64,
    pub memory_swap_mib: u64,
    pub pids: u32,
    pub nofile: u64,
}

impl<'de> Deserialize<'de> for EnvironmentResourceLimits {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Default, Deserialize)]
        #[serde(default, deny_unknown_fields)]
        struct ResourceLimitOverrides {
            cpus: Option<u16>,
            memory_mib: Option<u64>,
            memory_swap_mib: Option<u64>,
            pids: Option<u32>,
            nofile: Option<u64>,
        }

        let overrides = ResourceLimitOverrides::deserialize(deserializer)?;
        let defaults = Self::default();
        let memory_mib = overrides.memory_mib.unwrap_or(defaults.memory_mib);
        Ok(Self {
            cpus: overrides.cpus.unwrap_or(defaults.cpus),
            memory_mib,
            // Keep the safe no-additional-swap posture when memory alone is
            // overridden. Repositories can still opt into swap explicitly.
            memory_swap_mib: overrides.memory_swap_mib.unwrap_or(memory_mib),
            pids: overrides.pids.unwrap_or(defaults.pids),
            nofile: overrides.nofile.unwrap_or(defaults.nofile),
        })
    }
}

impl Default for EnvironmentResourceLimits {
    fn default() -> Self {
        Self {
            cpus: 4,
            memory_mib: 8 * 1024,
            memory_swap_mib: 8 * 1024,
            pids: 2_048,
            nofile: 8_192,
        }
    }
}

impl EnvironmentResourceLimits {
    pub fn validate(self) -> Result<(), String> {
        if self.cpus == 0 {
            return Err(String::from(
                "environment.resources.cpus must be greater than zero",
            ));
        }
        if self.memory_mib == 0 {
            return Err(String::from(
                "environment.resources.memory_mib must be greater than zero",
            ));
        }
        if self.memory_swap_mib < self.memory_mib {
            return Err(String::from(
                "environment.resources.memory_swap_mib must be greater than or equal to memory_mib",
            ));
        }
        if self.pids == 0 {
            return Err(String::from(
                "environment.resources.pids must be greater than zero",
            ));
        }
        if self.nofile == 0 {
            return Err(String::from(
                "environment.resources.nofile must be greater than zero",
            ));
        }
        Ok(())
    }
}

pub(crate) fn normalize_mise_tools(
    tools: &mut Vec<MiseToolRequirement>,
) -> Result<(), EnvironmentPlanError> {
    for tool in tools.iter_mut() {
        tool.tool = required_label("Mise tool", tool.tool.clone())?.to_ascii_lowercase();
        if !tool.tool.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':' | b'/')
        }) {
            return Err(EnvironmentPlanError::EmptyField("Mise tool"));
        }
        normalize_version_requirement(&mut tool.version)?;
        normalize_source(&mut tool.source)?;
    }
    tools.sort();
    tools.dedup();
    if let Some(duplicate) = tools.windows(2).find(|pair| pair[0].tool == pair[1].tool) {
        return Err(EnvironmentPlanError::DuplicateMiseTool(
            duplicate[0].tool.clone(),
        ));
    }
    Ok(())
}

pub(crate) fn normalize_debian_package_spec(value: String) -> Result<String, EnvironmentPlanError> {
    let value = required_label("Debian package", value)?;
    let (raw_name, version) = value
        .split_once('=')
        .map_or((value.as_str(), None), |(name, version)| {
            (name, Some(version))
        });
    let name = raw_name.to_ascii_lowercase();
    let valid_name = name
        .bytes()
        .next()
        .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        && name.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'+' | b'-' | b'.')
        });
    let valid_version = version.is_none_or(|version| {
        !version.is_empty()
            && version.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.' | b':' | b'~')
            })
    });
    if !valid_name || !valid_version {
        return Err(EnvironmentPlanError::EmptyField("Debian package"));
    }
    Ok(version.map_or(name.clone(), |version| format!("{name}={version}")))
}

pub(crate) fn normalize_debian_packages(
    packages: &mut Vec<DebianPackageRequirement>,
) -> Result<(), EnvironmentPlanError> {
    for package in packages.iter_mut() {
        package.package = normalize_debian_package_spec(package.package.clone())?;
        normalize_source(&mut package.source)?;
    }
    packages.sort();
    packages.dedup();
    Ok(())
}
