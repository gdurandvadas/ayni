use crate::application::{
    EnvRunOperation, EnvShellOperation, EnvShowOperation, OutputFormat, RepositoryOperation,
};
use ayni_core::{AdapterRegistry, EnvironmentLock};
use ayni_environment::TargetSelection;
use std::path::Path;
use std::process::ExitCode;

fn result(result: Result<String, ayni_environment::BackendError>) -> ExitCode {
    match result {
        Ok(message) => {
            println!("{message}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{}", error.message);
            ExitCode::from(error.code)
        }
    }
}

pub(crate) fn doctor(operation: RepositoryOperation, registry: &AdapterRegistry) -> ExitCode {
    result(with_current_plan(&operation.repo_root, registry, || {
        ayni_environment::doctor(&operation.repo_root)
    }))
}

pub(crate) fn build(operation: RepositoryOperation, registry: &AdapterRegistry) -> ExitCode {
    result(with_current_plan(&operation.repo_root, registry, || {
        ayni_environment::build(&operation.repo_root)
    }))
}

pub(crate) fn shell(operation: EnvShellOperation, registry: &AdapterRegistry) -> ExitCode {
    match ensure_current_plan(&operation.repo_root, registry) {
        Ok(()) => launch(
            &operation.repo_root,
            TargetSelection {
                language: operation.language,
                root: operation.root,
            },
            &[],
            true,
        ),
        Err(error) => render_error(error),
    }
}

pub(crate) fn run(operation: EnvRunOperation, registry: &AdapterRegistry) -> ExitCode {
    match ensure_current_plan(&operation.repo_root, registry) {
        Ok(()) => launch(
            &operation.repo_root,
            TargetSelection {
                language: operation.language,
                root: operation.root,
            },
            &operation.command,
            false,
        ),
        Err(error) => render_error(error),
    }
}

fn with_current_plan(
    repo_root: &Path,
    registry: &AdapterRegistry,
    operation: impl FnOnce() -> Result<String, ayni_environment::BackendError>,
) -> Result<String, ayni_environment::BackendError> {
    ensure_current_plan(repo_root, registry)?;
    operation()
}

fn ensure_current_plan(
    repo_root: &Path,
    registry: &AdapterRegistry,
) -> Result<(), ayni_environment::BackendError> {
    let canonical = repo_root
        .canonicalize()
        .map_err(|error| ayni_environment::BackendError {
            code: 2,
            message: format!(
                "failed to establish repository root {}: {error}",
                repo_root.display()
            ),
        })?;
    let lock = ayni_environment::read_lock(&canonical)?;
    let operation = EnvShowOperation {
        config: lock.repository().contract_path.clone().into(),
        repo_root: canonical,
        output: OutputFormat::Json,
    };
    let plan = crate::environment::build_plan(&operation, registry).map_err(|error| {
        ayni_environment::BackendError {
            code: error.code,
            message: format!(
                "environment lock is stale or unsupported: {}; run `ayni env lock`",
                error.message
            ),
        }
    })?;
    if plan.conflicts().is_empty() && plan_matches_lock(&plan, &lock) {
        Ok(())
    } else {
        Err(ayni_environment::BackendError {
            code: 3,
            message: String::from(
                "environment lock is stale because discovered requirements changed; run `ayni env lock`",
            ),
        })
    }
}

fn plan_matches_lock(plan: &ayni_core::EnvironmentPlan, lock: &EnvironmentLock) -> bool {
    if plan.targets().len() != lock.targets().len() {
        return false;
    }
    plan.targets()
        .iter()
        .zip(lock.targets())
        .all(|(plan, locked)| {
            plan.target == locked.target
                && plan.runtimes.len() == locked.runtimes.len()
                && plan
                    .runtimes
                    .iter()
                    .zip(&locked.runtimes)
                    .all(|(left, right)| {
                        left.runtime == right.runtime
                            && left.components == right.components
                            && left.targets == right.targets
                            && left.source.path == right.source.path
                    })
                && match (&plan.package_manager, &locked.package_manager) {
                    (None, None) => true,
                    (Some(left), Some(right)) => {
                        left.family == right.family
                            && left.ownership_root == right.ownership_root
                            && left.source.path == right.source.path
                    }
                    _ => false,
                }
                && plan.signal_tools.len() == locked.signal_tools.len()
                && plan
                    .signal_tools
                    .iter()
                    .zip(&locked.signal_tools)
                    .all(|(left, right)| {
                        left.tool == right.tool
                            && left.provider == right.provider
                            && left.scope == right.scope
                            && left.signals == right.signals
                            && left.source.path == right.source.path
                    })
                && plan.dependency_locks.len() == locked.dependency_locks.len()
                && plan
                    .dependency_locks
                    .iter()
                    .zip(&locked.dependency_locks)
                    .all(|(left, right)| {
                        left.path == right.path
                            && left.digest == right.digest
                            && left.owner_root == right.owner_root
                    })
        })
}

fn launch(repo_root: &Path, target: TargetSelection, command: &[String], shell: bool) -> ExitCode {
    match ayni_environment::launch(repo_root, &target, command, shell) {
        Ok(0) => ExitCode::SUCCESS,
        Ok(code) => {
            eprintln!("managed environment command exited with code {code}");
            ExitCode::from(4)
        }
        Err(error) => render_error(error),
    }
}

fn render_error(error: ayni_environment::BackendError) -> ExitCode {
    eprintln!("{}", error.message);
    ExitCode::from(error.code)
}
