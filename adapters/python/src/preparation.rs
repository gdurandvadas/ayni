use ayni_core::{
    AdapterError, DependencyPreparationCapability, DependencyPreparationPlan,
    DependencyPreparationRequest, Language, PreparationCommand, PreparationInput,
    PreparationOutput,
};
use std::collections::BTreeMap;
#[derive(Debug, Default)]
pub(crate) struct PythonDependencyPreparationCapability;
impl DependencyPreparationCapability for PythonDependencyPreparationCapability {
    fn language(&self) -> Language {
        Language::Python
    }
    fn prepare(
        &self,
        r: &DependencyPreparationRequest,
    ) -> Result<DependencyPreparationPlan, AdapterError> {
        let t = r.target();
        let m = t
            .package_manager
            .as_ref()
            .ok_or_else(|| err("Python preparation requires a package manager"))?;
        if m.family != "uv" {
            return Err(err(
                "Python dependency preparation supports uv/uv.lock only",
            ));
        }
        let owner = &m.ownership_root;
        let inputs = t
            .dependency_locks
            .iter()
            .filter(|x| x.owner_root == *owner)
            .map(|x| PreparationInput {
                path: x.path.clone(),
                digest: x.digest.clone(),
                owner_root: x.owner_root.clone(),
            })
            .collect::<Vec<_>>();
        let lock = if owner == "." {
            "uv.lock".into()
        } else {
            format!("{owner}/uv.lock")
        };
        if !inputs.iter().any(|x| x.path == lock) {
            return Err(err(format!("uv dependency preparation requires {lock}")));
        }
        let venv = if owner == "." {
            ".venv".into()
        } else {
            format!("{owner}/.venv")
        };
        DependencyPreparationPlan::new(
            t.target.clone(),
            inputs,
            vec![PreparationCommand::new(
                Language::Python,
                "uv",
                vec![
                    "sync".into(),
                    "--frozen".into(),
                    "--no-install-project".into(),
                ],
                owner,
                BTreeMap::from([("UV_CACHE_DIR".into(), "/home/ayni/.cache/uv".into())]),
            )?],
            vec![],
            vec![PreparationCommand::new(
                Language::Python,
                "uv",
                vec!["sync".into(), "--frozen".into(), "--offline".into()],
                owner,
                BTreeMap::new(),
            )?],
            vec![PreparationOutput {
                path: venv.clone(),
                mount_path: venv,
                mode: ayni_core::PreparationOutputMode::Fresh,
            }],
            BTreeMap::from([
                ("UV_CACHE_DIR".into(), "/home/ayni/.cache/uv".into()),
                ("UV_OFFLINE".into(), "true".into()),
                ("UV_FROZEN".into(), "true".into()),
                ("UV_NO_SYNC".into(), "true".into()),
                (
                    "VIRTUAL_ENV".into(),
                    if owner == "." {
                        "/workspace/.venv".into()
                    } else {
                        format!("/workspace/{owner}/.venv")
                    },
                ),
            ]),
        )
    }
}
fn err(e: impl std::fmt::Display) -> AdapterError {
    AdapterError::new(Language::Python, e.to_string())
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
            target: TargetIdentity::new(Language::Python, ".").expect("target"),
            workspace: None,
            package: None,
            runtimes: vec![],
            package_manager: Some(PackageManagerRequirement {
                family: family.into(),
                version: VersionRequirement::exact("0.6.0").expect("version"),
                ownership_root: ".".into(),
                source: RequirementSource::new(
                    "test",
                    "pyproject.toml",
                    None::<String>,
                    RequirementConfidence::Exact,
                )
                .expect("source"),
            }),
            signal_tools: vec![],
            system_requirements: vec![],
            dependency_locks: ["pyproject.toml", "uv.lock"]
                .into_iter()
                .map(|path| DependencyLockRequirement {
                    path: path.into(),
                    digest: format!("sha256:{}", "0".repeat(64)),
                    owner_root: ".".into(),
                    source: RequirementSource::new(
                        "test",
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
    fn plans_staged_uv_sync_and_rejects_non_uv() {
        let repo = TempDir::new().expect("repo");
        let plan = PythonDependencyPreparationCapability
            .prepare(
                &DependencyPreparationRequest::new(PathBuf::from(repo.path()), target("uv"))
                    .expect("request"),
            )
            .expect("plan");
        assert_eq!(
            plan.commands[0].args,
            ["sync", "--frozen", "--no-install-project"]
        );
        assert_eq!(
            plan.materialization_commands[0].args,
            ["sync", "--frozen", "--offline"]
        );
        assert_eq!(
            plan.outputs[0].mode,
            ayni_core::PreparationOutputMode::Fresh
        );
        assert_eq!(
            plan.execution_environment.get("UV_CACHE_DIR"),
            Some(&"/home/ayni/.cache/uv".into())
        );
        assert_eq!(
            plan.execution_environment.get("VIRTUAL_ENV"),
            Some(&"/workspace/.venv".into())
        );
        assert!(
            PythonDependencyPreparationCapability
                .prepare(
                    &DependencyPreparationRequest::new(
                        PathBuf::from(repo.path()),
                        target("poetry")
                    )
                    .expect("request")
                )
                .is_err()
        );
    }
}
