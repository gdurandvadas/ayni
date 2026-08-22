use ayni_core::{
    AdapterError, DependencyPreparationCapability, DependencyPreparationPlan,
    DependencyPreparationRequest, Language, PreparationCommand, PreparationInput,
    PreparationScaffold, sha256_hex,
};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

#[derive(Debug, Default)]
pub(crate) struct RustDependencyPreparationCapability;

impl DependencyPreparationCapability for RustDependencyPreparationCapability {
    fn language(&self) -> Language {
        Language::Rust
    }

    fn prepare(
        &self,
        request: &DependencyPreparationRequest,
    ) -> Result<DependencyPreparationPlan, AdapterError> {
        let target = request.target();
        let owner = target.workspace.as_deref().unwrap_or(&target.target.root);
        let inputs = target
            .dependency_locks
            .iter()
            .filter(|input| input.owner_root == owner)
            .map(|input| PreparationInput {
                path: input.path.clone(),
                digest: input.digest.clone(),
                owner_root: input.owner_root.clone(),
            })
            .collect::<Vec<_>>();
        if !inputs
            .iter()
            .any(|input| input.path == format!("{}/Cargo.lock", owner).trim_start_matches("./"))
        {
            return Err(AdapterError::new(
                Language::Rust,
                format!("Cargo dependency preparation requires {owner}/Cargo.lock"),
            ));
        }
        let scaffolds = cargo_scaffolds(request.repo_root(), &inputs)?;
        DependencyPreparationPlan::new(
            target.target.clone(),
            inputs,
            vec![PreparationCommand::new(
                Language::Rust,
                "cargo",
                vec![String::from("fetch"), String::from("--locked")],
                owner,
                BTreeMap::new(),
            )?],
            scaffolds,
            Vec::new(),
            Vec::new(),
            BTreeMap::from([
                // Docker Desktop bind mounts can lose Cargo artifacts under parallel rustc writes.
                (String::from("CARGO_BUILD_JOBS"), String::from("1")),
                (String::from("CARGO_NET_OFFLINE"), String::from("true")),
                (
                    String::from("CARGO_TARGET_DIR"),
                    cargo_target_directory(&target.target.root),
                ),
            ]),
        )
    }
}

fn cargo_target_directory(target_root: &str) -> String {
    let digest = sha256_hex(target_root);
    format!("/home/ayni/.cache/cargo/targets/{digest}")
}

fn cargo_scaffolds(
    repo_root: &Path,
    inputs: &[PreparationInput],
) -> Result<Vec<PreparationScaffold>, AdapterError> {
    let mut scaffolds = Vec::new();
    for input in inputs
        .iter()
        .filter(|input| input.path.ends_with("Cargo.toml"))
    {
        let content = fs::read_to_string(repo_root.join(&input.path)).map_err(|error| {
            AdapterError::new(
                Language::Rust,
                format!(
                    "failed to read preparation manifest {}: {error}",
                    input.path
                ),
            )
        })?;
        let manifest: toml::Value = toml::from_str(&content).map_err(|error| {
            AdapterError::new(
                Language::Rust,
                format!(
                    "failed to parse preparation manifest {}: {error}",
                    input.path
                ),
            )
        })?;
        if manifest.get("package").is_none() {
            continue;
        }
        let parent = Path::new(&input.path)
            .parent()
            .unwrap_or_else(|| Path::new(""));
        let mut targets = vec![String::from("src/lib.rs"), String::from("src/main.rs")];
        if let Some(path) = manifest
            .get("lib")
            .and_then(toml::Value::as_table)
            .and_then(|table| table.get("path"))
            .and_then(toml::Value::as_str)
        {
            targets.push(path.to_owned());
        }
        for section in ["bin", "example", "test", "bench"] {
            if let Some(entries) = manifest.get(section).and_then(toml::Value::as_array) {
                targets.extend(entries.iter().filter_map(|entry| {
                    entry
                        .as_table()
                        .and_then(|table| table.get("path"))
                        .and_then(toml::Value::as_str)
                        .map(str::to_owned)
                }));
            }
        }
        if let Some(path) = manifest
            .get("package")
            .and_then(toml::Value::as_table)
            .and_then(|table| table.get("build"))
            .and_then(toml::Value::as_str)
        {
            targets.push(path.to_owned());
        }
        scaffolds.extend(targets.into_iter().map(|target| PreparationScaffold {
            path: parent.join(target).to_string_lossy().replace('\\', "/"),
            content: String::new(),
        }));
    }
    scaffolds.sort();
    scaffolds.dedup();
    Ok(scaffolds)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ayni_core::{
        DependencyLockRequirement, RequirementConfidence, RequirementSource, TargetEnvironment,
        TargetIdentity,
    };
    use std::fs;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn target(lock: bool) -> TargetEnvironment {
        TargetEnvironment {
            target: TargetIdentity::new(Language::Rust, ".").expect("target"),
            workspace: None,
            package: None,
            runtimes: vec![],
            package_manager: None,
            signal_tools: vec![],
            system_requirements: vec![],
            dependency_locks: if lock {
                vec![
                    DependencyLockRequirement {
                        path: String::from("Cargo.toml"),
                        digest: format!("sha256:{}", "0".repeat(64)),
                        owner_root: String::from("."),
                        source: RequirementSource::new(
                            "cargo_manifest",
                            "Cargo.toml",
                            None::<String>,
                            RequirementConfidence::Exact,
                        )
                        .expect("source"),
                    },
                    DependencyLockRequirement {
                        path: String::from("Cargo.lock"),
                        digest: format!("sha256:{}", "1".repeat(64)),
                        owner_root: String::from("."),
                        source: RequirementSource::new(
                            "cargo_lock",
                            "Cargo.lock",
                            None::<String>,
                            RequirementConfidence::Exact,
                        )
                        .expect("source"),
                    },
                ]
            } else {
                vec![]
            },
        }
    }
    #[test]
    fn plans_locked_cargo_fetch_and_rejects_missing_lock() {
        let repo = TempDir::new().expect("repo");
        fs::write(
            repo.path().join("Cargo.toml"),
            "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\n",
        )
        .expect("manifest");
        let plan = ayni_adapters_common::environment::assert_dependency_preparation_conformance(
            &RustDependencyPreparationCapability,
            &DependencyPreparationRequest::new(PathBuf::from(repo.path()), target(true))
                .expect("request"),
        )
        .expect("plan");
        assert_eq!(plan.commands[0].program, "cargo");
        assert_eq!(plan.commands[0].args, ["fetch", "--locked"]);
        assert_eq!(
            plan.execution_environment.get("CARGO_NET_OFFLINE"),
            Some(&String::from("true"))
        );
        assert_eq!(
            plan.execution_environment.get("CARGO_BUILD_JOBS"),
            Some(&String::from("1"))
        );
        assert_eq!(
            plan.execution_environment.get("CARGO_TARGET_DIR"),
            Some(&cargo_target_directory("."))
        );
        assert!(
            RustDependencyPreparationCapability
                .prepare(
                    &DependencyPreparationRequest::new(PathBuf::from(repo.path()), target(false))
                        .expect("request")
                )
                .is_err()
        );
    }

    #[test]
    fn cargo_target_cache_is_stable_and_isolated_by_target_root() {
        let root = cargo_target_directory(".");
        assert_eq!(root, cargo_target_directory("."));
        assert_ne!(root, cargo_target_directory("crates/member"));
        assert!(root.starts_with("/home/ayni/.cache/cargo/targets/"));
        assert_eq!(
            root.trim_start_matches("/home/ayni/.cache/cargo/targets/")
                .len(),
            64
        );
    }
}
