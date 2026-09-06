//! Adapter-owned baselines. Catalogs remain authoritative for signal mappings.
use crate::{CatalogEntry, SignalKind, VersionRequirement};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolBaseline {
    Exact(&'static str),
    Toolchain,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolIntegration {
    /// Already represented by an environment runtime or package manager.
    Runtime,
    ToolchainComponent,
    Isolated {
        provider: &'static str,
    },
    ProjectDependency {
        package: &'static str,
    },
    GradlePlugin {
        plugin_ids: &'static [&'static str],
    },
    /// A bundled Gradle plugin whose library version is configured separately.
    GradleToolVersion {
        plugin_id: &'static str,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ManagedToolSpec {
    pub catalog_name: &'static str,
    pub baseline: ToolBaseline,
    pub integration: ToolIntegration,
}

impl ManagedToolSpec {
    pub const fn project(package: &'static str, version: &'static str) -> Self {
        Self {
            catalog_name: package,
            baseline: ToolBaseline::Exact(version),
            integration: ToolIntegration::ProjectDependency { package },
        }
    }

    pub const fn runtime(catalog_name: &'static str) -> Self {
        Self {
            catalog_name,
            baseline: ToolBaseline::Toolchain,
            integration: ToolIntegration::Runtime,
        }
    }

    pub fn exact_version(&self) -> Option<&'static str> {
        match self.baseline {
            ToolBaseline::Exact(version) => Some(version),
            ToolBaseline::Toolchain => None,
        }
    }

    pub fn plugin_ids(&self) -> &'static [&'static str] {
        match self.integration {
            ToolIntegration::GradlePlugin { plugin_ids } => plugin_ids,
            _ => &[],
        }
    }
}

/// Requires one entry for every catalog tool, including runtime tools, so an
/// added external tool cannot silently escape baseline coverage.
pub fn validate_managed_tools(
    catalog: &[CatalogEntry],
    tools: &[ManagedToolSpec],
) -> Result<(), String> {
    let catalog_names = catalog
        .iter()
        .map(|entry| entry.name)
        .collect::<BTreeSet<_>>();
    let names = tools
        .iter()
        .map(|tool| tool.catalog_name)
        .collect::<BTreeSet<_>>();
    if names != catalog_names || names.len() != tools.len() || catalog_names.len() != catalog.len()
    {
        return Err("managed tools must cover every catalog entry exactly once".into());
    }
    for tool in tools {
        validate_baseline(tool)?;
        validate_integration(tool.integration)?;
    }
    Ok(())
}

fn validate_baseline(tool: &ManagedToolSpec) -> Result<(), String> {
    match (tool.baseline, tool.integration) {
        (
            ToolBaseline::Toolchain,
            ToolIntegration::Runtime | ToolIntegration::ToolchainComponent,
        ) => Ok(()),
        (
            ToolBaseline::Exact(_),
            ToolIntegration::Runtime | ToolIntegration::ToolchainComponent,
        )
        | (ToolBaseline::Toolchain, _) => Err(
            "runtime components must follow their toolchain; external tools need exact baselines"
                .into(),
        ),
        (ToolBaseline::Exact(version), _) => VersionRequirement::exact(version)
            .map(|_| ())
            .map_err(|error| error.to_string()),
    }
}

fn validate_integration(integration: ToolIntegration) -> Result<(), String> {
    let labels = match integration {
        ToolIntegration::Runtime | ToolIntegration::ToolchainComponent => return Ok(()),
        ToolIntegration::Isolated { provider } => vec![provider],
        ToolIntegration::ProjectDependency { package } => vec![package],
        ToolIntegration::GradlePlugin { plugin_ids } => plugin_ids.to_vec(),
        ToolIntegration::GradleToolVersion { plugin_id } => vec![plugin_id],
    };
    if labels.is_empty()
        || labels
            .iter()
            .any(|label| label.trim().is_empty() || label.chars().any(char::is_control))
        || labels.iter().collect::<BTreeSet<_>>().len() != labels.len()
    {
        return Err("tool integration needs unique nonempty identifiers".into());
    }
    Ok(())
}

/// Select external tools only for signals still using their default collector.
/// Alternative providers (such as coverage plugins) are narrowed by the adapter.
/// Existing, disabled tools are deliberately not represented as removals.
pub fn select_managed_tools<'a>(
    catalog: &[CatalogEntry],
    tools: &'a [ManagedToolSpec],
    default_signals: &BTreeSet<SignalKind>,
) -> Result<Vec<(&'a ManagedToolSpec, Vec<SignalKind>)>, String> {
    validate_managed_tools(catalog, tools)?;
    Ok(tools
        .iter()
        .filter_map(|tool| {
            if tool.integration == ToolIntegration::Runtime {
                return None;
            }
            let entry = catalog
                .iter()
                .find(|entry| entry.name == tool.catalog_name)?;
            let signals = entry
                .for_signals
                .iter()
                .copied()
                .filter(|signal| default_signals.contains(signal))
                .collect::<BTreeSet<_>>();
            (!signals.is_empty()).then(|| (tool, signals.into_iter().collect()))
        })
        .collect())
}

#[cfg(test)]
#[path = "tool_baseline_tests.rs"]
mod tests;
