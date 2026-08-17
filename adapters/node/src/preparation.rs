use ayni_core::{
    AdapterError, DependencyPreparationCapability, DependencyPreparationPlan,
    DependencyPreparationRequest, Language, PreparationCommand, PreparationInput,
    PreparationOutput,
};
use std::collections::{BTreeMap, BTreeSet};
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
        let (lock_name, install_args, materialization_args) = match manager.family.as_str() {
            "npm" => (
                "package-lock.json",
                vec!["ci", "--ignore-scripts", "--no-audit", "--no-fund"],
                vec!["rebuild", "--offline", "--no-audit", "--no-fund"],
            ),
            "pnpm" => (
                "pnpm-lock.yaml",
                vec!["install", "--frozen-lockfile", "--ignore-scripts"],
                vec!["rebuild"],
            ),
            unsupported => {
                return Err(AdapterError::new(
                    Language::Node,
                    format!(
                        "Node dependency preparation supports npm and pnpm; {unsupported} is unsupported"
                    ),
                ));
            }
        };
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
            lock_name.to_owned()
        } else {
            format!("{owner}/{lock_name}")
        };
        if !inputs.iter().any(|input| input.path == lock_path) {
            return Err(AdapterError::new(
                Language::Node,
                format!(
                    "{} dependency preparation requires {lock_path}",
                    manager.family
                ),
            ));
        }
        reject_unstaged_local_dependencies(request.repo_root(), &inputs)?;
        // The backend executes these commands only in a staged copy. pnpm
        // creates package-local node_modules trees whose symlinks are required
        // when commands execute from workspace members, so every declared tree
        // is seeded and mounted rather than only the workspace-owner tree.
        let outputs = node_module_outputs(owner, &inputs, &manager.family);
        let mut commands = vec![PreparationCommand::new(
            Language::Node,
            manager.family.clone(),
            install_args.into_iter().map(String::from).collect(),
            owner,
            BTreeMap::new(),
        )?];
        if manager.family == "pnpm" {
            // pnpm removes empty package-local node_modules directories during
            // install. Recreate every declared output afterwards so the image
            // can seed even workspace members that have no local dependencies.
            commands.extend(prepare_output_directories(&outputs)?);
        }
        let mut execution_environment =
            BTreeMap::from([(String::from("npm_config_offline"), String::from("true"))]);
        if manager.family == "pnpm" {
            // The prepared modules tree is authoritative and mounted read-only
            // with respect to the checkout. pnpm 11 otherwise tries to run an
            // implicit install before `pnpm exec`, which cannot safely rewrite
            // that managed tree during quality execution.
            execution_environment.insert(
                String::from("PNPM_CONFIG_VERIFY_DEPS_BEFORE_RUN"),
                String::from("false"),
            );
        }
        DependencyPreparationPlan::new(
            target.target.clone(),
            inputs,
            commands,
            Vec::new(),
            vec![PreparationCommand::new(
                Language::Node,
                manager.family.clone(),
                materialization_args.into_iter().map(String::from).collect(),
                owner,
                BTreeMap::new(),
            )?],
            outputs,
            execution_environment,
        )
    }
}

fn node_module_outputs(
    owner: &str,
    inputs: &[PreparationInput],
    manager: &str,
) -> Vec<PreparationOutput> {
    let owner_modules = join_repository_path(owner, "node_modules");
    let mut paths = BTreeSet::from([owner_modules]);
    if manager == "pnpm" {
        for input in inputs
            .iter()
            .filter(|input| input.path.ends_with("package.json"))
        {
            let parent = std::path::Path::new(&input.path)
                .parent()
                .and_then(std::path::Path::to_str)
                .unwrap_or(".");
            paths.insert(join_repository_path(parent, "node_modules"));
        }
    }
    paths
        .into_iter()
        .map(|path| PreparationOutput {
            mount_path: path.clone(),
            path,
            mode: ayni_core::PreparationOutputMode::Seeded,
        })
        .collect()
}

fn prepare_output_directories(
    outputs: &[PreparationOutput],
) -> Result<Vec<PreparationCommand>, AdapterError> {
    outputs
        .iter()
        .map(|output| {
            let cwd = output
                .path
                .strip_suffix("/node_modules")
                .filter(|path| !path.is_empty())
                .unwrap_or(".");
            PreparationCommand::new(
                Language::Node,
                "mkdir",
                vec![String::from("-p"), String::from("node_modules")],
                cwd,
                BTreeMap::new(),
            )
        })
        .collect()
}

fn join_repository_path(parent: &str, child: &str) -> String {
    if parent == "." || parent.is_empty() {
        child.to_owned()
    } else {
        format!("{parent}/{child}")
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
            dependency_locks: [
                "package.json",
                if family == "pnpm" {
                    "pnpm-lock.yaml"
                } else {
                    "package-lock.json"
                },
            ]
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
    fn plans_npm_and_pnpm_dependency_preparation() {
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
        assert!(
            !plan
                .execution_environment
                .contains_key("PNPM_CONFIG_VERIFY_DEPS_BEFORE_RUN")
        );
        let pnpm_request =
            DependencyPreparationRequest::new(PathBuf::from(repo.path()), target("pnpm"))
                .expect("request");
        let pnpm_plan = NodeDependencyPreparationCapability
            .prepare(&pnpm_request)
            .expect("pnpm plan");
        let install = pnpm_plan
            .commands
            .iter()
            .find(|command| command.program == "pnpm")
            .expect("pnpm install");
        assert_eq!(install.program, "pnpm");
        assert_eq!(
            install.args,
            ["install", "--frozen-lockfile", "--ignore-scripts"]
        );
        assert_eq!(
            pnpm_plan
                .commands
                .last()
                .map(|command| command.program.as_str()),
            Some("mkdir")
        );
        assert_eq!(pnpm_plan.materialization_commands[0].program, "pnpm");
        assert_eq!(
            pnpm_plan
                .execution_environment
                .get("PNPM_CONFIG_VERIFY_DEPS_BEFORE_RUN"),
            Some(&String::from("false"))
        );

        fs::create_dir_all(repo.path().join("packages/app")).expect("member directory");
        fs::write(
            repo.path().join("packages/app/package.json"),
            r#"{"name":"app","dependencies":{"vitest":"3.2.4"}}"#,
        )
        .expect("member manifest");
        let mut workspace_target = target("pnpm");
        workspace_target
            .dependency_locks
            .push(DependencyLockRequirement {
                path: String::from("packages/app/package.json"),
                digest: format!("sha256:{}", "1".repeat(64)),
                owner_root: String::from("."),
                source: RequirementSource::new(
                    "node_manifest",
                    "packages/app/package.json",
                    None::<String>,
                    RequirementConfidence::Exact,
                )
                .expect("source"),
            });
        let workspace_request =
            DependencyPreparationRequest::new(PathBuf::from(repo.path()), workspace_target)
                .expect("workspace request");
        let workspace_plan = NodeDependencyPreparationCapability
            .prepare(&workspace_request)
            .expect("workspace pnpm plan");
        assert!(
            workspace_plan
                .outputs
                .iter()
                .any(|output| output.mount_path == "packages/app/node_modules")
        );
        assert!(workspace_plan.commands.iter().any(|command| {
            command.program == "mkdir"
                && command.cwd == "packages/app"
                && command.args == ["-p", "node_modules"]
        }));
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
