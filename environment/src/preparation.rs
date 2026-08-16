use crate::BackendError;
use crate::runtime::{WORKSPACE, target_environment};
use ayni_core::{
    DependencyPreparationPlan, EnvironmentLock, PreparationOutput, PreparationOutputMode,
};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) const INPUT_ROOT: &str = "/tmp/ayni/repository";
pub(crate) const SEED_ROOT: &str = "/opt/ayni/dependencies";
const PREPARATION_IMPLEMENTATION_VERSION: &str = "2";

pub(crate) fn dockerfile_fragment(
    lock: &EnvironmentLock,
    plans: &[DependencyPreparationPlan],
) -> Result<String, BackendError> {
    if plans.is_empty() {
        return Ok(String::new());
    }
    let mut output = String::from(
        "FROM ayni-runtime AS ayni-preparation\nCOPY --chown=10001:10001 repository /tmp/ayni/repository\n",
    );
    for plan in ordered_plans(plans) {
        let target = lock
            .targets()
            .iter()
            .find(|target| target.target == plan.target)
            .ok_or_else(|| {
                BackendError::environment(format!(
                    "dependency preparation target {}:{} is absent from the lock",
                    plan.target.language, plan.target.root
                ))
            })?;
        let activation = target_environment(target)?;
        for command in &plan.commands {
            output.push_str("WORKDIR ");
            output.push_str(&docker_path(INPUT_ROOT, &command.cwd));
            output.push('\n');
            let mut argv = vec![String::from("env")];
            argv.extend(
                activation
                    .iter()
                    .map(|(name, value)| format!("{name}={value}")),
            );
            argv.extend(
                command
                    .environment
                    .iter()
                    .map(|(name, value)| format!("{name}={value}")),
            );
            argv.push(command.program.clone());
            argv.extend(command.args.clone());
            output.push_str("RUN ");
            output.push_str(&serde_json::to_string(&argv).expect("argv serialization"));
            output.push('\n');
        }
    }
    output.push_str(
        "FROM ayni-runtime\nUSER root\nCOPY --from=ayni-preparation /home/ayni/.cache /home/ayni/.cache\nRUN chmod -R a+rX /home/ayni/.cache\nUSER ayni\n",
    );
    for plan in ordered_plans(plans) {
        for prepared in &plan.outputs {
            if prepared.mode == PreparationOutputMode::Fresh {
                continue;
            }
            output.push_str("COPY --from=ayni-preparation ");
            output.push_str(&docker_path(INPUT_ROOT, &prepared.path));
            output.push(' ');
            output.push_str(&format!("{SEED_ROOT}/{}", output_key(prepared)));
            output.push('\n');
        }
    }
    Ok(output)
}

pub(crate) fn stage_inputs(
    repo_root: &Path,
    context_root: &Path,
    plans: &[DependencyPreparationPlan],
) -> Result<(), BackendError> {
    let destination_root = context_root.join("repository");
    fs::create_dir(&destination_root).map_err(|error| {
        BackendError::execution(format!("failed to create staged dependency input: {error}"))
    })?;
    stage_locked_inputs(repo_root, &destination_root, plans)?;
    stage_scaffolds(&destination_root, plans)
}

fn stage_locked_inputs(
    repo_root: &Path,
    destination_root: &Path,
    plans: &[DependencyPreparationPlan],
) -> Result<(), BackendError> {
    let mut staged = BTreeMap::new();
    for plan in ordered_plans(plans) {
        for input in &plan.inputs {
            if let Some(previous) = staged.insert(input.path.clone(), input.digest.clone())
                && previous != input.digest
            {
                return Err(BackendError::environment(format!(
                    "dependency input {} has conflicting locked digests",
                    input.path
                )));
            }
            stage_locked_input(repo_root, destination_root, input)?;
        }
    }
    Ok(())
}

fn stage_locked_input(
    repo_root: &Path,
    destination_root: &Path,
    input: &ayni_core::PreparationInput,
) -> Result<(), BackendError> {
    let source = contained_file(repo_root, &input.path)?;
    let bytes = fs::read(&source).map_err(|error| {
        BackendError::environment(format!(
            "failed to read dependency input {}: {error}",
            input.path
        ))
    })?;
    let actual = format!("sha256:{:x}", Sha256::digest(&bytes));
    if actual != input.digest {
        return Err(BackendError::environment(format!(
            "dependency input {} changed; run `ayni env lock`",
            input.path
        )));
    }
    let destination = destination_root.join(&input.path);
    create_parent(&destination, "staged dependency input")?;
    if destination.exists() {
        return Ok(());
    }
    fs::write(&destination, bytes).map_err(|error| {
        BackendError::execution(format!(
            "failed to stage dependency input {}: {error}",
            input.path
        ))
    })
}

fn stage_scaffolds(
    destination_root: &Path,
    plans: &[DependencyPreparationPlan],
) -> Result<(), BackendError> {
    let mut generated = BTreeMap::new();
    for plan in ordered_plans(plans) {
        for scaffold in &plan.scaffolds {
            if let Some(previous) =
                generated.insert(scaffold.path.clone(), scaffold.content.clone())
                && previous != scaffold.content
            {
                return Err(BackendError::environment(format!(
                    "preparation scaffold {} has conflicting contents",
                    scaffold.path
                )));
            }
            stage_scaffold(destination_root, scaffold)?;
        }
    }
    Ok(())
}

fn stage_scaffold(
    destination_root: &Path,
    scaffold: &ayni_core::PreparationScaffold,
) -> Result<(), BackendError> {
    let destination = destination_root.join(&scaffold.path);
    if destination.exists() {
        return Ok(());
    }
    create_parent(&destination, "preparation scaffold")?;
    fs::write(&destination, scaffold.content.as_bytes()).map_err(|error| {
        BackendError::execution(format!(
            "failed to write preparation scaffold {}: {error}",
            scaffold.path
        ))
    })
}

fn create_parent(path: &Path, description: &str) -> Result<(), BackendError> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    fs::create_dir_all(parent).map_err(|error| {
        BackendError::execution(format!("failed to create {description} directory: {error}"))
    })
}

pub(crate) fn managed_environments(
    lock: &EnvironmentLock,
    plans: &[DependencyPreparationPlan],
    state_home: &str,
) -> Result<String, BackendError> {
    let mut environments = BTreeMap::<String, BTreeMap<String, String>>::new();
    for plan in ordered_plans(plans) {
        let target = lock
            .targets()
            .iter()
            .find(|target| target.target == plan.target)
            .ok_or_else(|| BackendError::environment("preparation target is absent from lock"))?;
        let mut environment = target_environment(target)?
            .into_iter()
            .collect::<BTreeMap<_, _>>();
        environment.extend(resolved_execution_environment(plan, state_home));
        environments.insert(target_key(&plan.target), environment);
    }
    serde_json::to_string(&environments).map_err(|error| {
        BackendError::execution(format!(
            "failed to serialize managed target environments: {error}"
        ))
    })
}

pub(crate) fn resolved_execution_environment(
    plan: &DependencyPreparationPlan,
    state_home: &str,
) -> BTreeMap<String, String> {
    let target_hash = format!("{:x}", Sha256::digest(target_key(&plan.target)));
    plan.execution_environment
        .iter()
        .map(|(name, value)| {
            let value = value.strip_prefix("@generated/").map_or_else(
                || value.clone(),
                |relative| format!("{state_home}/targets/{target_hash}/{relative}"),
            );
            (name.clone(), value)
        })
        .collect()
}

pub(crate) fn preparation_digest(
    plans: &[DependencyPreparationPlan],
) -> Result<String, BackendError> {
    let plans = ordered_plans(plans);
    let bytes =
        serde_json::to_vec(&(PREPARATION_IMPLEMENTATION_VERSION, plans)).map_err(|error| {
            BackendError::execution(format!(
                "failed to serialize dependency preparation: {error}"
            ))
        })?;
    Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
}

pub(crate) fn unique_outputs(plans: &[DependencyPreparationPlan]) -> Vec<PreparationOutput> {
    let mut outputs = BTreeSet::new();
    for plan in plans {
        outputs.extend(plan.outputs.iter().cloned());
    }
    outputs.into_iter().collect()
}

pub(crate) fn output_key(output: &PreparationOutput) -> String {
    format!(
        "{:x}",
        Sha256::digest(format!("{}\0{}", output.path, output.mount_path))
    )
}

pub(crate) fn target_key(target: &ayni_core::TargetIdentity) -> String {
    format!("{}:{}", target.language, target.root)
}

fn ordered_plans(plans: &[DependencyPreparationPlan]) -> Vec<&DependencyPreparationPlan> {
    let mut plans = plans.iter().collect::<Vec<_>>();
    plans.sort_by(|left, right| left.target.cmp(&right.target));
    plans
}

fn docker_path(root: &str, relative: &str) -> String {
    if relative == "." {
        root.to_owned()
    } else {
        format!("{root}/{relative}")
    }
}

fn contained_file(repo_root: &Path, relative: &str) -> Result<PathBuf, BackendError> {
    let source = repo_root.join(relative);
    let canonical = source.canonicalize().map_err(|error| {
        BackendError::environment(format!(
            "failed to inspect dependency input {relative}: {error}"
        ))
    })?;
    if canonical.starts_with(repo_root) && canonical.is_file() {
        Ok(canonical)
    } else {
        Err(BackendError::environment(format!(
            "dependency input escapes the repository or is not a file: {relative}"
        )))
    }
}

pub(crate) fn workspace_mount(output: &PreparationOutput) -> String {
    if output.mount_path == "." {
        WORKSPACE.to_owned()
    } else {
        format!("{WORKSPACE}/{}", output.mount_path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ayni_core::{Language, PreparationCommand, PreparationInput, TargetIdentity};

    fn plan(root: &str, commands: &[&str]) -> DependencyPreparationPlan {
        DependencyPreparationPlan {
            target: TargetIdentity::new(Language::Rust, root).expect("target"),
            inputs: vec![PreparationInput {
                path: format!("{root}/Cargo.lock"),
                digest: format!("sha256:{}", "0".repeat(64)),
                owner_root: root.into(),
            }],
            commands: commands
                .iter()
                .map(|program| PreparationCommand {
                    program: (*program).into(),
                    args: Vec::new(),
                    cwd: root.into(),
                    environment: BTreeMap::new(),
                })
                .collect(),
            scaffolds: Vec::new(),
            materialization_commands: Vec::new(),
            outputs: Vec::new(),
            execution_environment: BTreeMap::new(),
        }
    }

    #[test]
    fn preparation_digest_orders_targets_but_preserves_command_semantics() {
        let first = plan("one", &["cargo", "rustc"]);
        let second = plan("two", &["cargo"]);
        assert_eq!(
            preparation_digest(&[first.clone(), second.clone()]).expect("digest"),
            preparation_digest(&[second, first.clone()]).expect("digest")
        );
        assert_ne!(
            preparation_digest(std::slice::from_ref(&first)).expect("digest"),
            preparation_digest(&[plan("one", &["rustc", "cargo"])]).expect("digest")
        );
        assert_ne!(
            preparation_digest(std::slice::from_ref(&first)).expect("digest"),
            preparation_digest(&[plan("one", &["cargo", "rustc", "cargo"])]).expect("digest")
        );
    }
}
