use crate::environment::{
    EnvironmentPlanError, RequirementSource, VersionRequirement, normalize_source,
    normalize_version_requirement, required_label,
};
use serde::{Deserialize, Serialize};

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
