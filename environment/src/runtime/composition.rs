//! Repository-wide activation without promoting project-local tool directories.
use super::{mise_version_variable, target_environment};
use crate::{BackendError, signal_tool_coordinate};
use ayni_core::EnvironmentLock;
use std::collections::{BTreeMap, BTreeSet};

type Requirements = BTreeMap<String, BTreeMap<String, BTreeSet<String>>>;

pub(super) fn repository_environment(
    lock: &EnvironmentLock,
) -> Result<BTreeMap<String, String>, BackendError> {
    let mut requirements = Requirements::new();
    let mut variables = BTreeMap::new();
    for target in lock.targets() {
        let owner = format!("{}:{}", target.target.language, target.target.root);
        for runtime in &target.runtimes {
            add(
                &mut requirements,
                &runtime.runtime,
                &runtime.version,
                &owner,
            );
        }
        if let Some(manager) = &target.package_manager {
            add(&mut requirements, &manager.family, &manager.version, &owner);
        }
        for tool in &target.signal_tools {
            if let Some(coordinate) =
                signal_tool_coordinate(tool.scope, &tool.tool, &tool.provider)?
            {
                add(&mut requirements, &coordinate, &tool.version, &owner);
            }
        }
        variables.extend(target_environment(target)?);
    }
    for tool in lock.tools() {
        add(
            &mut requirements,
            &tool.tool,
            &tool.version,
            "repository tools",
        );
        // Provider coordinates are selected by the locked Mise configuration;
        // runtime-style names additionally support explicit activation variables.
        if !tool.tool.contains(':') {
            variables.insert(mise_version_variable(&tool.tool)?, tool.version.clone());
        }
    }
    validate_requirements(&requirements)?;
    Ok(variables)
}

fn add(requirements: &mut Requirements, tool: &str, version: &str, owner: &str) {
    requirements
        .entry(tool.into())
        .or_default()
        .entry(version.into())
        .or_default()
        .insert(owner.into());
}

fn validate_requirements(requirements: &Requirements) -> Result<(), BackendError> {
    let conflicts = requirements
        .iter()
        .filter(|(_, versions)| versions.len() > 1)
        .map(|(tool, versions)| {
            let owners = versions
                .iter()
                .map(|(version, owners)| {
                    format!(
                        "{version} ({})",
                        owners.iter().cloned().collect::<Vec<_>>().join(", ")
                    )
                })
                .collect::<Vec<_>>()
                .join("; ");
            format!("{tool}: {owners}")
        })
        .collect::<Vec<_>>();
    if conflicts.is_empty() {
        Ok(())
    } else {
        Err(BackendError::input(format!(
            "repository environment has conflicting activation requirements: {}. Align the requirements or select --language and --root explicitly",
            conflicts.join("; ")
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_versions_compose_and_conflicts_name_every_owner() {
        let mut requirements = Requirements::new();
        add(&mut requirements, "node", "24.14.0", "node:frontend");
        add(&mut requirements, "node", "24.14.0", "node:worker");
        add(&mut requirements, "rust", "1.97.1", "rust:backend");
        validate_requirements(&requirements).unwrap();
        add(&mut requirements, "node", "22.0.0", "node:legacy");
        let error = validate_requirements(&requirements).unwrap_err().message;
        for expected in [
            "24.14.0",
            "22.0.0",
            "node:frontend",
            "node:worker",
            "node:legacy",
            "--language and --root",
        ] {
            assert!(error.contains(expected), "{error}");
        }
    }
}
