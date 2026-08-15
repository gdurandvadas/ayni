use ayni_core::{
    AdapterError, DependencyPreparationCapability, DependencyPreparationPlan,
    DependencyPreparationRequest, Language, PreparationCommand, PreparationInput,
};
use std::collections::BTreeMap;

#[derive(Debug, Default)]
pub(crate) struct GoDependencyPreparationCapability;

impl DependencyPreparationCapability for GoDependencyPreparationCapability {
    fn language(&self) -> Language {
        Language::Go
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
            .map(|input| PreparationInput {
                path: input.path.clone(),
                digest: input.digest.clone(),
                owner_root: input.owner_root.clone(),
            })
            .collect::<Vec<_>>();
        let module_sum = prefixed(&target.target.root, "go.sum");
        if !inputs.iter().any(|input| input.path == module_sum)
            && module_has_requirements(request.repo_root(), &target.target.root)?
        {
            return Err(AdapterError::new(
                Language::Go,
                format!(
                    "Go dependency preparation requires {module_sum} when go.mod declares dependencies"
                ),
            ));
        }
        if owner != target.target.root
            && !inputs
                .iter()
                .any(|input| input.path == prefixed(owner, "go.work"))
        {
            return Err(AdapterError::new(
                Language::Go,
                format!(
                    "Go workspace dependency preparation requires {}",
                    prefixed(owner, "go.work")
                ),
            ));
        }
        let cache_environment = BTreeMap::from([
            ("GOMODCACHE".into(), "/home/ayni/.cache/go/pkg/mod".into()),
            ("GOCACHE".into(), "/home/ayni/.cache/go/build".into()),
            ("GOPATH".into(), "/home/ayni/.cache/go".into()),
            ("GOTOOLCHAIN".into(), "local".into()),
            ("GOWORK".into(), "off".into()),
        ]);
        let mut execution_environment = cache_environment.clone();
        execution_environment.insert("GOPROXY".into(), "off".into());
        execution_environment.insert("GOFLAGS".into(), "-mod=readonly".into());
        if target.workspace.is_some() {
            execution_environment.remove("GOWORK");
        }
        // The backend executes this only against staged, digest-checked manifests.
        DependencyPreparationPlan::new(
            target.target.clone(),
            inputs,
            vec![PreparationCommand::new(
                Language::Go,
                "go",
                vec!["mod".into(), "download".into(), "all".into()],
                &target.target.root,
                cache_environment,
            )?],
            Vec::new(),
            Vec::new(),
            Vec::new(),
            execution_environment,
        )
    }
}

fn module_has_requirements(repo_root: &std::path::Path, root: &str) -> Result<bool, AdapterError> {
    let path = repo_root.join(prefixed(root, "go.mod"));
    let content = std::fs::read_to_string(&path).map_err(|cause| {
        AdapterError::new(
            Language::Go,
            format!("failed to inspect {}: {cause}", path.display()),
        )
    })?;
    Ok(content.lines().any(|line| {
        let line = line
            .split_once("//")
            .map_or(line, |(before, _)| before)
            .trim();
        line.starts_with("require ") || line == "require("
    }))
}

fn prefixed(root: &str, file: &str) -> String {
    if root == "." {
        file.into()
    } else {
        format!("{root}/{file}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ayni_adapters_common::environment::assert_dependency_preparation_conformance;
    use ayni_core::{
        DependencyLockRequirement, RequirementConfidence, RequirementSource, TargetEnvironment,
        TargetIdentity,
    };
    use std::{fs, path::PathBuf};
    use tempfile::TempDir;

    fn target(include_sum: bool) -> TargetEnvironment {
        let mut locks = vec!["go.mod"];
        if include_sum {
            locks.push("go.sum");
        }
        TargetEnvironment {
            target: TargetIdentity::new(Language::Go, ".").expect("target"),
            workspace: None,
            package: Some("example.com/fixture".into()),
            runtimes: vec![],
            package_manager: None,
            signal_tools: vec![],
            system_requirements: vec![],
            dependency_locks: locks
                .into_iter()
                .map(|path| DependencyLockRequirement {
                    path: path.into(),
                    digest: format!("sha256:{}", "0".repeat(64)),
                    owner_root: ".".into(),
                    source: RequirementSource::new(
                        "go_input",
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
    fn plans_readonly_module_download_into_external_caches() {
        let repo = TempDir::new().expect("repo");
        fs::write(repo.path().join("go.mod"), "module example.com/fixture\n").expect("mod");
        fs::write(repo.path().join("go.sum"), "example v1 h1:x\n").expect("sum");
        let plan = assert_dependency_preparation_conformance(
            &GoDependencyPreparationCapability,
            &DependencyPreparationRequest::new(PathBuf::from(repo.path()), target(true))
                .expect("request"),
        )
        .expect("plan");
        assert_eq!(plan.commands[0].args, ["mod", "download", "all"]);
        assert_eq!(
            plan.execution_environment.get("GOMODCACHE"),
            Some(&"/home/ayni/.cache/go/pkg/mod".into())
        );
        assert!(plan.outputs.is_empty());
    }

    #[test]
    fn rejects_missing_checksum_lock() {
        let repo = TempDir::new().expect("repo");
        let request = DependencyPreparationRequest::new(PathBuf::from(repo.path()), target(false))
            .expect("request");
        assert!(GoDependencyPreparationCapability.prepare(&request).is_err());
    }
}
