use crate::application::{
    EnvRunOperation, EnvShellOperation, EnvShowOperation, OutputFormat, RepositoryOperation,
};
use ayni_core::AdapterRegistry;
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
    if plan.conflicts().is_empty() && ayni_environment::plan_matches_lock(&plan, &lock) {
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
