use ayni_core::{
    AdapterError, DependencyPreparationCapability, DependencyPreparationPlan,
    DependencyPreparationRequest, Language, PreparationCommand, PreparationInput,
    PreparationOutput,
};
use std::collections::BTreeMap;
use std::fs;

#[derive(Debug, Default)]
pub(crate) struct NodeDependencyPreparationCapability;

impl DependencyPreparationCapability for NodeDependencyPreparationCapability {
    fn language(&self) -> Language {
        Language::Node
    }

    fn prepare(
        &self,
        request: &DependencyPreparationRequest,
    ) -> Result<DependencyPreparationPlan, AdapterError> {
        let target = request.target();
        let manager = target.package_manager.as_ref().ok_or_else(|| {
            AdapterError::new(
                Language::Node,
                "Node dependency preparation requires a package manager",
            )
        })?;
        if manager.family != "npm" {
            return Err(AdapterError::new(
                Language::Node,
                format!(
                    "Node dependency preparation supports npm/package-lock.json only; {} is unsupported",
                    manager.family
                ),
            ));
        }
        let owner = &manager.ownership_root;
        let inputs = target
            .dependency_locks
            .iter()
            .filter(|input| input.owner_root == *owner)
            .map(|input| PreparationInput {
                path: input.path.clone(),
                digest: input.digest.clone(),
                owner_root: input.owner_root.clone(),
            })
            .collect::<Vec<_>>();
        let lock_path = if owner == "." {
            "package-lock.json".to_owned()
        } else {
            format!("{owner}/package-lock.json")
        };
        if !inputs.iter().any(|input| input.path == lock_path) {
            return Err(AdapterError::new(
                Language::Node,
                format!("npm dependency preparation requires {lock_path}"),
            ));
        }
        reject_unstaged_local_dependencies(request.repo_root(), &inputs)?;
        // The backend must execute this only in a staged copy of `inputs`; npm
        // materializes node_modules, which must never be written to the checkout.
        let node_modules = if owner == "." {
            String::from("node_modules")
        } else {
            format!("{owner}/node_modules")
        };
        DependencyPreparationPlan::new(
            target.target.clone(),
            inputs,
            vec![PreparationCommand::new(
                Language::Node,
                "npm",
                vec![
                    String::from("ci"),
                    String::from("--ignore-scripts"),
                    String::from("--no-audit"),
                    String::from("--no-fund"),
                ],
                owner,
                BTreeMap::new(),
            )?],
            Vec::new(),
            vec![PreparationCommand::new(
                Language::Node,
                "npm",
                vec![
                    String::from("rebuild"),
                    String::from("--offline"),
                    String::from("--no-audit"),
                    String::from("--no-fund"),
                ],
                owner,
                BTreeMap::new(),
            )?],
            vec![PreparationOutput {
                path: node_modules.clone(),
                mount_path: node_modules,
                mode: ayni_core::PreparationOutputMode::Seeded,
            }],
            BTreeMap::from([(String::from("npm_config_offline"), String::from("true"))]),
        )
    }
}

fn reject_unstaged_local_dependencies(
    repo_root: &std::path::Path,
    inputs: &[PreparationInput],
) -> Result<(), AdapterError> {
    for input in inputs.iter().filter(|input| {
        input.path.ends_with("package.json") || input.path.ends_with("package-lock.json")
    }) {
        let bytes = fs::read(repo_root.join(&input.path)).map_err(|error| {
            AdapterError::new(
                Language::Node,
                format!(
                    "failed to read npm preparation input {}: {error}",
                    input.path
                ),
            )
        })?;
        let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|error| {
            AdapterError::new(
                Language::Node,
                format!(
                    "failed to parse npm preparation input {}: {error}",
                    input.path
                ),
            )
        })?;
        let local_dependency = if input.path.ends_with("package-lock.json") {
            lock_has_local_dependency(&value)
        } else {
            manifest_has_local_dependency(&value)
        };
        if local_dependency {
            return Err(AdapterError::new(
                Language::Node,
                format!(
                    "npm dependency preparation does not yet support file: or link: dependencies referenced by {}",
                    input.path
                ),
            ));
        }
    }
    Ok(())
}

fn manifest_has_local_dependency(value: &serde_json::Value) -> bool {
    [
        "dependencies",
        "devDependencies",
        "optionalDependencies",
        "peerDependencies",
    ]
    .iter()
    .filter_map(|field| value.get(field).and_then(serde_json::Value::as_object))
    .flat_map(|dependencies| dependencies.values())
    .any(is_local_specifier)
}

fn lock_has_local_dependency(value: &serde_json::Value) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    object.iter().any(|(key, value)| {
        matches!(key.as_str(), "version" | "resolved") && is_local_specifier(value)
            || matches!(
                key.as_str(),
                "dependencies"
                    | "devDependencies"
                    | "optionalDependencies"
                    | "peerDependencies"
                    | "requires"
            ) && dependency_map_has_local(value)
            || lock_has_local_dependency(value)
    })
}

fn dependency_map_has_local(value: &serde_json::Value) -> bool {
    value
        .as_object()
        .is_some_and(|dependencies| dependencies.values().any(is_local_specifier))
}

fn is_local_specifier(value: &serde_json::Value) -> bool {
    value
        .as_str()
        .is_some_and(|value| value.starts_with("file:") || value.starts_with("link:"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ayni_core::{
        DependencyLockRequirement, PackageManagerRequirement, RequirementConfidence,
        RequirementSource, TargetEnvironment, TargetIdentity, VersionRequirement,
    };
    use std::path::PathBuf;
    use tempfile::TempDir;
    fn target(family: &str) -> TargetEnvironment {
        TargetEnvironment {
            target: TargetIdentity::new(Language::Node, ".").expect("target"),
            workspace: None,
            package: None,
            runtimes: vec![],
            package_manager: Some(PackageManagerRequirement {
                family: family.into(),
                version: VersionRequirement::exact("10.0.0").expect("version"),
                ownership_root: String::from("."),
                source: RequirementSource::new(
                    "package_json_package_manager",
                    "package.json",
                    None::<String>,
                    RequirementConfidence::Exact,
                )
                .expect("source"),
            }),
            signal_tools: vec![],
            system_requirements: vec![],
            dependency_locks: ["package.json", "package-lock.json"]
                .into_iter()
                .map(|path| DependencyLockRequirement {
                    path: path.into(),
                    digest: format!("sha256:{}", "0".repeat(64)),
                    owner_root: String::from("."),
                    source: RequirementSource::new(
                        "node_input",
                        path,
                        None::<String>,
                        RequirementConfidence::Exact,
                    )
                    .expect("source"),
                })
                .collect(),
        }
    }
    #[test]
    fn plans_npm_ci_only_and_rejects_other_managers() {
        let repo = TempDir::new().expect("repo");
        fs::write(repo.path().join("package.json"), r#"{"name":"fixture"}"#).expect("manifest");
        fs::write(
            repo.path().join("package-lock.json"),
            r#"{"lockfileVersion":3,"packages":{}}"#,
        )
        .expect("lock");
        let request = DependencyPreparationRequest::new(PathBuf::from(repo.path()), target("npm"))
            .expect("request");
        let plan = ayni_adapters_common::environment::assert_dependency_preparation_conformance(
            &NodeDependencyPreparationCapability,
            &request,
        )
        .expect("plan");
        assert_eq!(plan.commands[0].program, "npm");
        assert_eq!(plan.commands[0].args[0], "ci");
        assert_eq!(plan.materialization_commands[0].args[0], "rebuild");
        assert_eq!(plan.outputs[0].mount_path, "node_modules");
        assert_eq!(
            plan.execution_environment.get("npm_config_offline"),
            Some(&String::from("true"))
        );
        let unsupported =
            DependencyPreparationRequest::new(PathBuf::from(repo.path()), target("pnpm"))
                .expect("request");
        assert!(
            NodeDependencyPreparationCapability
                .prepare(&unsupported)
                .is_err()
        );
    }

    #[test]
    fn ignores_unrelated_file_like_metadata() {
        let repo = TempDir::new().expect("repo");
        fs::write(
            repo.path().join("package.json"),
            r#"{"scripts":{"show":"echo file:fixture"},"description":"link: metadata"}"#,
        )
        .expect("manifest");
        fs::write(
            repo.path().join("package-lock.json"),
            r#"{"lockfileVersion":3,"packages":{"":{"description":"file: metadata"}}}"#,
        )
        .expect("lock");
        let request = DependencyPreparationRequest::new(PathBuf::from(repo.path()), target("npm"))
            .expect("request");
        NodeDependencyPreparationCapability
            .prepare(&request)
            .expect("metadata must not be treated as a dependency");
    }

    #[test]
    fn rejects_local_dependencies_that_are_not_staged() {
        let repo = TempDir::new().expect("repo");
        fs::write(
            repo.path().join("package.json"),
            r#"{"dependencies":{"local":"file:../local"}}"#,
        )
        .expect("manifest");
        fs::write(
            repo.path().join("package-lock.json"),
            r#"{"lockfileVersion":3,"packages":{}}"#,
        )
        .expect("lock");
        let request = DependencyPreparationRequest::new(PathBuf::from(repo.path()), target("npm"))
            .expect("request");
        let error = NodeDependencyPreparationCapability
            .prepare(&request)
            .expect_err("local dependency must be rejected");
        assert!(error.to_string().contains("file: or link:"));

        fs::write(repo.path().join("package.json"), r#"{"name":"fixture"}"#).expect("manifest");
        fs::write(
            repo.path().join("package-lock.json"),
            r#"{"lockfileVersion":3,"packages":{"node_modules/local":{"resolved":"link:../local"}}}"#,
        )
        .expect("lock");
        let error = NodeDependencyPreparationCapability
            .prepare(&request)
            .expect_err("locked local dependency must be rejected");
        assert!(error.to_string().contains("package-lock.json"));
    }
}
