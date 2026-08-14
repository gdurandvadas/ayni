use crate::application::{
    CheckOperation, EnvRunOperation, EnvShellOperation, EnvShowOperation, OutputFormat,
    RepositoryOperation,
};
use ayni_core::{
    AdapterRegistry, DependencyPreparationPlan, DependencyPreparationRequest, EnvironmentPlan,
};
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
    result((|| {
        let (root, plan) = current_plan(&operation.repo_root, registry)?;
        let preparations = dependency_preparations(&root, registry, &plan)?;
        ayni_environment::doctor_prepared(&root, &preparations)
    })())
}

pub(crate) fn build(operation: RepositoryOperation, registry: &AdapterRegistry) -> ExitCode {
    result((|| {
        let (root, plan) = current_plan(&operation.repo_root, registry)?;
        let preparations = dependency_preparations(&root, registry, &plan)?;
        ayni_environment::build_prepared(&root, &preparations)
    })())
}

pub(crate) fn check(operation: CheckOperation, registry: &AdapterRegistry) -> ExitCode {
    let result = (|| {
        let repo_root = operation
            .config
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let (root, plan) = current_plan(repo_root, registry)?;
        let preparations = dependency_preparations(&root, registry, &plan)?;
        let config =
            operation
                .config
                .canonicalize()
                .map_err(|error| ayni_environment::BackendError {
                    code: 2,
                    message: format!(
                        "failed to resolve environment contract {}: {error}",
                        operation.config.display()
                    ),
                })?;
        let relative = config
            .strip_prefix(&root)
            .map_err(|_| ayni_environment::BackendError {
                code: 2,
                message: String::from("environment contract escapes the repository root"),
            })?;
        let container_config = format!(
            "/workspace/{}",
            relative.to_string_lossy().replace('\\', "/")
        );
        let mut command = vec![
            String::from("check"),
            String::from("--host"),
            String::from("--config"),
            container_config,
            String::from("--output"),
            match operation.output {
                OutputFormat::Human => String::from("human"),
                OutputFormat::Json => String::from("json"),
                OutputFormat::Markdown => String::from("markdown"),
            },
        ];
        if operation.debug {
            command.push(String::from("--debug"));
        }
        ayni_environment::launch_repository_prepared(&root, &preparations, &command)
    })();
    match result {
        Ok(code @ 0..=4) => ExitCode::from(code as u8),
        Ok(code) => {
            eprintln!("managed check exited with unsupported code {code}");
            ExitCode::from(4)
        }
        Err(error) => render_error(error),
    }
}

pub(crate) fn shell(operation: EnvShellOperation, registry: &AdapterRegistry) -> ExitCode {
    match current_plan(&operation.repo_root, registry).and_then(|(root, plan)| {
        let preparations = dependency_preparations(&root, registry, &plan)?;
        Ok((root, preparations))
    }) {
        Ok((root, preparations)) => launch(
            &root,
            TargetSelection {
                language: operation.language,
                root: operation.root,
            },
            &[],
            true,
            &preparations,
        ),
        Err(error) => render_error(error),
    }
}

pub(crate) fn run(operation: EnvRunOperation, registry: &AdapterRegistry) -> ExitCode {
    match current_plan(&operation.repo_root, registry).and_then(|(root, plan)| {
        let preparations = dependency_preparations(&root, registry, &plan)?;
        Ok((root, preparations))
    }) {
        Ok((root, preparations)) => launch(
            &root,
            TargetSelection {
                language: operation.language,
                root: operation.root,
            },
            &operation.command,
            false,
            &preparations,
        ),
        Err(error) => render_error(error),
    }
}

fn current_plan(
    repo_root: &Path,
    registry: &AdapterRegistry,
) -> Result<(std::path::PathBuf, EnvironmentPlan), ayni_environment::BackendError> {
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
        repo_root: canonical.clone(),
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
        Ok((canonical, plan))
    } else {
        Err(ayni_environment::BackendError {
            code: 3,
            message: String::from(
                "environment lock is stale because discovered requirements changed; run `ayni env lock`",
            ),
        })
    }
}

fn dependency_preparations(
    repo_root: &Path,
    registry: &AdapterRegistry,
    plan: &EnvironmentPlan,
) -> Result<Vec<DependencyPreparationPlan>, ayni_environment::BackendError> {
    plan.targets()
        .iter()
        .map(|target| {
            let adapter = registry
                .adapters()
                .iter()
                .find(|adapter| adapter.language() == target.target.language)
                .ok_or_else(|| ayni_environment::BackendError {
                    code: 3,
                    message: format!(
                        "no adapter can prepare dependencies for {}",
                        target.target.language
                    ),
                })?;
            let request =
                DependencyPreparationRequest::new(repo_root.to_path_buf(), target.clone())
                    .map_err(|error| ayni_environment::BackendError {
                        code: 3,
                        message: error.to_string(),
                    })?;
            adapter
                .prepare_dependencies(&request)
                .map_err(|error| ayni_environment::BackendError {
                    code: 3,
                    message: error.to_string(),
                })
        })
        .collect()
}

fn launch(
    repo_root: &Path,
    target: TargetSelection,
    command: &[String],
    shell: bool,
    preparations: &[DependencyPreparationPlan],
) -> ExitCode {
    match ayni_environment::launch_prepared(repo_root, &target, command, shell, preparations) {
        Ok(0) => ExitCode::SUCCESS,
        Ok(code @ 1..=255) => {
            eprintln!("managed environment command exited with code {code}");
            ExitCode::from(code as u8)
        }
        Ok(code) => {
            eprintln!("managed environment command returned invalid exit code {code}");
            ExitCode::from(4)
        }
        Err(error) => render_error(error),
    }
}

fn render_error(error: ayni_environment::BackendError) -> ExitCode {
    eprintln!("{}", error.message);
    ExitCode::from(error.code)
}
