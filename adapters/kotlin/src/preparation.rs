use ayni_core::{
    AdapterError, DependencyPreparationCapability, DependencyPreparationPlan,
    DependencyPreparationRequest, Language, PreparationCommand, PreparationInput,
    PreparationScaffold,
};
use std::collections::BTreeMap;

const RESOLVE_SCRIPT: &str = ".ayni-gradle-resolve.init.gradle";
const RESOLVE_TASK: &str = "ayniResolveDependencies";
const MANAGED_OUTPUT_ROOT: &str = "AYNI_GRADLE_OUTPUT_ROOT";
const RESOLVE_SCRIPT_CONTENT: &str = r#"gradle.projectsEvaluated {
    allprojects { project ->
        project.tasks.register("ayniResolveDependencies") {
            doLast {
                project.configurations.findAll {
                    it.canBeResolved && it.name != "kotlinNativeBundleConfiguration"
                }.each { configuration ->
                    configuration.resolve()
                }
            }
        }
    }
}
"#;

#[derive(Debug, Default)]
pub(crate) struct KotlinDependencyPreparationCapability;

impl DependencyPreparationCapability for KotlinDependencyPreparationCapability {
    fn language(&self) -> Language {
        Language::Kotlin
    }

    fn prepare(
        &self,
        request: &DependencyPreparationRequest,
    ) -> Result<DependencyPreparationPlan, AdapterError> {
        let target = request.target();
        let manager = target.package_manager.as_ref().ok_or_else(|| {
            AdapterError::new(Language::Kotlin, "Kotlin preparation requires Gradle")
        })?;
        if manager.family != "gradle" {
            return Err(AdapterError::new(
                Language::Kotlin,
                format!(
                    "Kotlin dependency preparation requires Gradle; {} is unsupported",
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
        for required in [
            "gradlew",
            "gradle/wrapper/gradle-wrapper.jar",
            "gradle/wrapper/gradle-wrapper.properties",
        ] {
            let path = prefixed(owner, required);
            if !inputs.iter().any(|input| input.path == path) {
                return Err(AdapterError::new(
                    Language::Kotlin,
                    format!("Gradle dependency preparation requires {path}"),
                ));
            }
        }
        if !inputs.iter().any(|input| {
            input.path.ends_with("gradle.lockfile")
                || input.path.contains("gradle/dependency-locks/")
                    && input.path.ends_with(".lockfile")
        }) {
            return Err(AdapterError::new(
                Language::Kotlin,
                "Gradle dependency preparation requires committed dependency locks",
            ));
        }

        let scaffold_path = prefixed(owner, RESOLVE_SCRIPT);
        let cache_environment =
            BTreeMap::from([("GRADLE_USER_HOME".into(), "/home/ayni/.cache/gradle".into())]);
        DependencyPreparationPlan::new(
            target.target.clone(),
            inputs,
            vec![PreparationCommand::new(
                Language::Kotlin,
                "sh",
                vec![
                    "gradlew".into(),
                    "--no-daemon".into(),
                    "--console=plain".into(),
                    "--stacktrace".into(),
                    "--init-script".into(),
                    RESOLVE_SCRIPT.into(),
                    RESOLVE_TASK.into(),
                ],
                owner,
                cache_environment,
            )?],
            vec![PreparationScaffold {
                path: scaffold_path,
                content: RESOLVE_SCRIPT_CONTENT.into(),
            }],
            Vec::new(),
            Vec::new(),
            BTreeMap::from([
                ("GRADLE_USER_HOME".into(), "/home/ayni/.cache/gradle".into()),
                ("AYNI_GRADLE_OFFLINE".into(), "1".into()),
                (
                    MANAGED_OUTPUT_ROOT.into(),
                    managed_output_root(&target.target.root),
                ),
            ]),
        )
    }
}

fn managed_output_root(target_root: &str) -> String {
    let encoded = target_root
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("/workspace/.ayni/quality/kotlin/{encoded}")
}

fn prefixed(root: &str, path: &str) -> String {
    if root == "." {
        path.into()
    } else {
        format!("{root}/{path}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ayni_adapters_common::environment::assert_dependency_preparation_conformance;
    use ayni_core::{
        DependencyLockRequirement, PackageManagerRequirement, RequirementConfidence,
        RequirementSource, TargetEnvironment, TargetIdentity, VersionRequirement,
    };
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn target() -> TargetEnvironment {
        let paths = [
            "gradlew",
            "gradle/wrapper/gradle-wrapper.jar",
            "gradle/wrapper/gradle-wrapper.properties",
            "settings.gradle.kts",
            "build.gradle.kts",
            "gradle.lockfile",
        ];
        TargetEnvironment {
            target: TargetIdentity::new(Language::Kotlin, ".").expect("target"),
            workspace: None,
            package: None,
            runtimes: Vec::new(),
            package_manager: Some(PackageManagerRequirement {
                family: "gradle".into(),
                version: VersionRequirement::exact("8.10.2").expect("version"),
                ownership_root: ".".into(),
                source: RequirementSource::new(
                    "wrapper",
                    "gradle/wrapper/gradle-wrapper.properties",
                    None::<String>,
                    RequirementConfidence::Exact,
                )
                .expect("source"),
            }),
            signal_tools: Vec::new(),
            system_requirements: Vec::new(),
            dependency_locks: paths
                .into_iter()
                .map(|path| DependencyLockRequirement {
                    path: path.into(),
                    digest: format!("sha256:{}", "0".repeat(64)),
                    owner_root: ".".into(),
                    source: RequirementSource::new(
                        "input",
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
    fn plans_locked_gradle_cache_warming_without_checkout_outputs() {
        let repo = TempDir::new().expect("repo");
        let plan = assert_dependency_preparation_conformance(
            &KotlinDependencyPreparationCapability,
            &DependencyPreparationRequest::new(PathBuf::from(repo.path()), target())
                .expect("request"),
        )
        .expect("plan");
        assert_eq!(plan.commands[0].program, "sh");
        assert_eq!(plan.commands[0].args[0], "gradlew");
        assert!(plan.commands[0].args.contains(&RESOLVE_TASK.into()));
        assert_eq!(plan.scaffolds[0].path, RESOLVE_SCRIPT);
        assert!(
            plan.scaffolds[0]
                .content
                .contains("it.name != \"kotlinNativeBundleConfiguration\"")
        );
        assert!(plan.outputs.is_empty());
        assert_eq!(
            plan.execution_environment.get("AYNI_GRADLE_OFFLINE"),
            Some(&"1".into())
        );
        assert_eq!(
            plan.execution_environment.get(MANAGED_OUTPUT_ROOT),
            Some(&"/workspace/.ayni/quality/kotlin/2e".into())
        );
    }
}
