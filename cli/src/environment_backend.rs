use crate::application::{
    CheckOperation, EnvRunOperation, EnvShellOperation, EnvShowOperation, ImpactOperation,
    OutputFormat, RepositoryOperation, VerifyOperation,
};
use ayni_core::{
    AdapterRegistry, DependencyPreparationPlan, DependencyPreparationRequest, EnvironmentLock,
    EnvironmentPlan,
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
        Err(error) => render_error(error),
    }
}

pub(crate) fn doctor(operation: RepositoryOperation, registry: &AdapterRegistry) -> ExitCode {
    result((|| {
        let (root, plan) = current_plan(&operation.repo_root, None, registry)?;
        let preparations = dependency_preparations(&root, registry, &plan)?;
        ayni_environment::doctor_prepared(&root, &preparations)
    })())
}

pub(crate) fn build(operation: RepositoryOperation, registry: &AdapterRegistry) -> ExitCode {
    result((|| {
        let (root, plan) = current_plan(&operation.repo_root, None, registry)?;
        let preparations = dependency_preparations(&root, registry, &plan)?;
        ayni_environment::build_prepared(&root, &preparations)
    })())
}

pub(crate) fn check(operation: CheckOperation, registry: &AdapterRegistry) -> ExitCode {
    managed_quality_result(
        "check",
        (|| {
            let (root, preparations, container_config) =
                prepared_quality_environment(&operation.config, registry)?;
            let mut command = vec![
                String::from("check"),
                String::from("--host"),
                String::from("--config"),
                container_config,
                String::from("--output"),
                output_name(operation.output).to_owned(),
            ];
            if operation.debug {
                command.push(String::from("--debug"));
            }
            ayni_environment::launch_repository_prepared(&root, &preparations, &command)
        })(),
    )
}

pub(crate) fn verify(operation: VerifyOperation, registry: &AdapterRegistry) -> ExitCode {
    managed_quality_result(
        "verify",
        (|| {
            let (root, preparations, container_config) =
                prepared_quality_environment(&operation.config, registry)?;
            let command = managed_verify_command(&operation, container_config);
            ayni_environment::launch_repository_prepared(&root, &preparations, &command)
        })(),
    )
}

pub(crate) fn impact_run(operation: ImpactOperation, registry: &AdapterRegistry) -> ExitCode {
    managed_quality_result(
        "impact run",
        (|| {
            let (root, preparations, container_config) =
                prepared_quality_environment(&operation.config, registry)?;
            let command = managed_impact_command(&operation, container_config);
            ayni_environment::launch_repository_prepared(&root, &preparations, &command)
        })(),
    )
}

fn managed_impact_command(operation: &ImpactOperation, container_config: String) -> Vec<String> {
    let mut command = vec![
        String::from("impact"),
        String::from("run"),
        String::from("--host"),
        String::from("--base"),
        operation.base.clone(),
        String::from("--config"),
        container_config,
        String::from("--output"),
        output_name(operation.output).to_owned(),
    ];
    if operation.debug {
        command.push(String::from("--debug"));
    }
    command
}

fn managed_verify_command(operation: &VerifyOperation, container_config: String) -> Vec<String> {
    let mut command = vec![
        String::from("verify"),
        crate::analysis::signal_kind_slug(operation.signal).to_owned(),
        String::from("--host"),
        String::from("--config"),
        container_config,
        String::from("--output"),
        output_name(operation.output).to_owned(),
    ];
    if let Some(language) = operation.language {
        command.extend([String::from("--language"), language.as_str().to_owned()]);
    }
    if let Some(root) = &operation.root {
        command.extend([String::from("--root"), root.clone()]);
    }
    if let Some(file) = &operation.file {
        command.extend([String::from("--file"), file.clone()]);
    }
    if let Some(package) = &operation.package {
        command.extend([String::from("--package"), package.clone()]);
    }
    if let Some(name) = &operation.name {
        command.extend([String::from("--name"), name.clone()]);
    }
    if operation.debug {
        command.push(String::from("--debug"));
    }
    command
}

fn output_name(output: OutputFormat) -> &'static str {
    match output {
        OutputFormat::Human => "human",
        OutputFormat::Json => "json",
        OutputFormat::Markdown => "markdown",
    }
}

fn prepared_quality_environment(
    config: &Path,
    registry: &AdapterRegistry,
) -> Result<
    (std::path::PathBuf, Vec<DependencyPreparationPlan>, String),
    ayni_environment::BackendError,
> {
    let repo_root = config
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let (root, plan) = current_plan(repo_root, Some(config), registry)?;
    let preparations = dependency_preparations(&root, registry, &plan)?;
    let config = config.canonicalize().map_err(|error| {
        ayni_environment::BackendError::input(format!(
            "failed to resolve environment contract {}: {error}",
            config.display()
        ))
    })?;
    let relative = config.strip_prefix(&root).map_err(|_| {
        ayni_environment::BackendError::input(String::from(
            "environment contract escapes the repository root",
        ))
    })?;
    let container_config = format!("./{}", relative.to_string_lossy().replace('\\', "/"));
    Ok((root, preparations, container_config))
}

fn managed_quality_result(
    operation: &str,
    result: Result<i32, ayni_environment::BackendError>,
) -> ExitCode {
    match result {
        Ok(code @ 0..=4) => ExitCode::from(code as u8),
        Ok(code) => {
            eprintln!("managed {operation} exited with unsupported code {code}");
            ExitCode::from(4)
        }
        Err(error) => render_error(error),
    }
}

pub(crate) fn shell(operation: EnvShellOperation, registry: &AdapterRegistry) -> ExitCode {
    match current_plan(&operation.repo_root, None, registry).and_then(|(root, plan)| {
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
    match current_plan(&operation.repo_root, None, registry).and_then(|(root, plan)| {
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
    requested_contract: Option<&Path>,
    registry: &AdapterRegistry,
) -> Result<(std::path::PathBuf, EnvironmentPlan), ayni_environment::BackendError> {
    let canonical = repo_root.canonicalize().map_err(|error| {
        ayni_environment::BackendError::input(format!(
            "failed to establish repository root {}: {error}",
            repo_root.display()
        ))
    })?;
    let lock = ayni_environment::read_lock(&canonical)?;
    if let Some(requested_contract) = requested_contract {
        ensure_requested_contract_matches_lock(&canonical, requested_contract, &lock)?;
    }
    let operation = EnvShowOperation {
        config: lock.repository().contract_path.clone().into(),
        repo_root: canonical.clone(),
        output: OutputFormat::Json,
    };
    let plan = crate::environment::build_plan(&operation, registry).map_err(|error| {
        ayni_environment::BackendError {
            kind: match error.kind {
                crate::application_error::ApplicationErrorKind::InvalidInput => {
                    ayni_environment::BackendErrorKind::Input
                }
                crate::application_error::ApplicationErrorKind::Environment => {
                    ayni_environment::BackendErrorKind::Environment
                }
                crate::application_error::ApplicationErrorKind::Execution => {
                    ayni_environment::BackendErrorKind::Execution
                }
            },
            message: format!(
                "environment lock is stale or unsupported: {}; run `ayni env lock`",
                error.message
            ),
        }
    })?;
    if plan.conflicts().is_empty() && ayni_environment::plan_matches_lock(&plan, &lock) {
        Ok((canonical, plan))
    } else {
        Err(ayni_environment::BackendError::environment(String::from(
            "environment lock is stale because discovered requirements changed; run `ayni env lock`",
        )))
    }
}

fn ensure_requested_contract_matches_lock(
    repo_root: &Path,
    requested_contract: &Path,
    lock: &EnvironmentLock,
) -> Result<(), ayni_environment::BackendError> {
    let requested = requested_contract.canonicalize().map_err(|error| {
        ayni_environment::BackendError::input(format!(
            "failed to resolve requested contract {}: {error}",
            requested_contract.display()
        ))
    })?;
    let locked = repo_root.join(&lock.repository().contract_path);
    let locked = locked.canonicalize().map_err(|error| {
        ayni_environment::BackendError::environment(format!(
            "locked contract {} is unavailable: {error}; run `ayni env lock`",
            lock.repository().contract_path
        ))
    })?;
    if requested != locked {
        return Err(ayni_environment::BackendError::environment(format!(
            "managed execution requires --config to match the lock-bound contract {} (digest {}); run `ayni env lock` after changing the contract",
            lock.repository().contract_path,
            lock.repository().contract_digest,
        )));
    }
    Ok(())
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
                .ok_or_else(|| {
                    ayni_environment::BackendError::environment(format!(
                        "no adapter can prepare dependencies for {}",
                        target.target.language
                    ))
                })?;
            let request =
                DependencyPreparationRequest::new(repo_root.to_path_buf(), target.clone())
                    .map_err(|error| {
                        ayni_environment::BackendError::environment(error.to_string())
                    })?;
            adapter
                .prepare_dependencies(&request)
                .map_err(|error| ayni_environment::BackendError::environment(error.to_string()))
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
    crate::application_error::render_error(error.into())
}

#[cfg(test)]
mod tests {
    use super::{managed_impact_command, managed_verify_command};
    use crate::application::{ExecutionMode, ImpactOperation, OutputFormat, VerifyOperation};
    use ayni_core::{Language, SignalKind};
    use std::path::PathBuf;

    #[test]
    fn managed_verify_forwards_the_complete_focused_request() {
        let operation = VerifyOperation {
            signal: SignalKind::Test,
            config: PathBuf::from("./.ayni.toml"),
            language: Some(Language::Node),
            root: Some(String::from("apps/web")),
            file: Some(String::from("apps/web/src/cart.test.ts")),
            package: None,
            name: Some(String::from("updates cart")),
            output: OutputFormat::Markdown,
            execution_mode: ExecutionMode::Managed,
            debug: true,
        };

        assert_eq!(
            managed_verify_command(&operation, String::from("./.ayni.toml")),
            [
                "verify",
                "test",
                "--host",
                "--config",
                "./.ayni.toml",
                "--output",
                "markdown",
                "--language",
                "node",
                "--root",
                "apps/web",
                "--file",
                "apps/web/src/cart.test.ts",
                "--name",
                "updates cart",
                "--debug",
            ]
            .map(String::from)
        );
    }

    #[test]
    fn managed_impact_forwards_explicit_change_identity_and_output() {
        let operation = ImpactOperation {
            config: PathBuf::from("./.ayni.toml"),
            base: String::from("feature/base"),
            output: OutputFormat::Json,
            execution_mode: ExecutionMode::Managed,
            debug: true,
        };

        assert_eq!(
            managed_impact_command(&operation, String::from("./.ayni.toml")),
            [
                "impact",
                "run",
                "--host",
                "--base",
                "feature/base",
                "--config",
                "./.ayni.toml",
                "--output",
                "json",
                "--debug",
            ]
            .map(String::from)
        );
    }
}
