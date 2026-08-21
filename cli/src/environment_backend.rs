use crate::application::{
    CapabilityAuthorization, CheckOperation, EnvPruneOperation, EnvRunOperation, EnvShellOperation,
    EnvShowOperation, EnvStorageOperation, ImpactOperation, OutputFormat, RepositoryOperation,
    VerifyOperation,
};
use ayni_core::{
    AdapterRegistry, DependencyPreparationPlan, DependencyPreparationRequest, DockerAccess,
    EnvironmentLock, EnvironmentPlan, NetworkAccess,
};
use ayni_environment::TargetSelection;
use std::fmt::Write as _;
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

pub(crate) fn storage(operation: EnvStorageOperation, registry: &AdapterRegistry) -> ExitCode {
    let report = (|| {
        let (root, plan) = current_plan(&operation.repo_root, None, registry)?;
        let preparations = dependency_preparations(&root, registry, &plan)?;
        ayni_environment::storage_report_prepared(&root, &preparations)
    })();
    match report {
        Ok(report) => match operation.output {
            OutputFormat::Human => {
                print!("{}", render_storage_report(&report));
                ExitCode::SUCCESS
            }
            OutputFormat::Json => render_json(&report),
            OutputFormat::Markdown => unreachable!("env storage does not accept Markdown output"),
        },
        Err(error) => render_error(error),
    }
}

pub(crate) fn prune(operation: EnvPruneOperation, registry: &AdapterRegistry) -> ExitCode {
    let result = (|| {
        let (root, plan) = current_plan(&operation.repo_root, None, registry)?;
        let preparations = dependency_preparations(&root, registry, &plan)?;
        ayni_environment::prune_storage_prepared(
            &root,
            &preparations,
            operation.apply,
            operation.images,
        )
    })();
    match result {
        Ok(result) => {
            let complete = result.complete();
            let rendered = match operation.output {
                OutputFormat::Human => {
                    print!("{}", render_storage_prune(&result));
                    ExitCode::SUCCESS
                }
                OutputFormat::Json => render_json(&result),
                OutputFormat::Markdown => {
                    unreachable!("env prune does not accept Markdown output")
                }
            };
            if rendered == ExitCode::SUCCESS && !complete {
                ExitCode::from(4)
            } else {
                rendered
            }
        }
        Err(error) => render_error(error),
    }
}

fn render_json(value: &impl serde::Serialize) -> ExitCode {
    match serde_json::to_string_pretty(value) {
        Ok(output) => {
            println!("{output}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("failed to render environment storage data: {error}");
            ExitCode::from(4)
        }
    }
}

fn render_storage_report(report: &ayni_environment::StorageReport) -> String {
    let mut output = String::new();
    writeln!(output, "Ayni environment storage ({})", report.engine).expect("string write");
    writeln!(output, "Expected image tag: {}", report.expected_image_tag).expect("string write");
    writeln!(
        output,
        "Current image present: {}",
        if report.current_image_present {
            "yes"
        } else {
            "no; run `ayni env build` to create it"
        }
    )
    .expect("string write");
    writeln!(
        output,
        "Images: {} ({} cumulative)",
        report.images.len(),
        format_bytes(report.image_cumulative_size_bytes)
    )
    .expect("string write");
    for image in &report.images {
        let state = if image.current {
            "current"
        } else if image.prune_candidate {
            "stale"
        } else {
            "legacy"
        };
        let name = image.tags.first().map_or(image.id.as_str(), String::as_str);
        writeln!(
            output,
            "  {state:<7} {:>10}  {name}",
            format_bytes(image.cumulative_size_bytes)
        )
        .expect("string write");
    }
    writeln!(
        output,
        "State root: {} total logical data under {}",
        format_bytes(report.state_root_logical_size_bytes),
        report.state_root
    )
    .expect("string write");
    writeln!(
        output,
        "Classified environment state: {} path(s), {} logical data",
        report.state_generations.len(),
        format_bytes(report.classified_state_logical_size_bytes),
    )
    .expect("string write");
    for generation in &report.state_generations {
        let state = if generation.current {
            "current"
        } else {
            "stale"
        };
        writeln!(
            output,
            "  {state:<7} {:>10}  {}",
            format_bytes(generation.logical_size_bytes),
            generation.path
        )
        .expect("string write");
    }
    writeln!(
        output,
        "Unclassified state: {} (included in state-root total; reported, never pruned)",
        format_bytes(report.unclassified_state_logical_size_bytes)
    )
    .expect("string write");
    writeln!(
        output,
        "Image sizes are cumulative, not unique or reclaimable; shared layers and engine build cache are not attributed."
    )
    .expect("string write");
    writeln!(
        output,
        "Image deletion scope is engine-wide across repositories; repository-local state is reported separately."
    )
    .expect("string write");
    output
}

fn render_storage_prune(result: &ayni_environment::StoragePruneResult) -> String {
    let image_candidates = result
        .report
        .images
        .iter()
        .filter(|image| image.prune_candidate)
        .collect::<Vec<_>>();
    let state_candidates = result
        .report
        .state_generations
        .iter()
        .filter(|generation| generation.prune_candidate)
        .collect::<Vec<_>>();
    let mut output = String::new();
    if result.applied {
        writeln!(output, "Ayni storage prune applied").expect("string write");
    } else {
        writeln!(output, "Ayni storage prune dry run").expect("string write");
    }
    writeln!(
        output,
        "Repository-local state candidates: {} managed-state path(s)",
        state_candidates.len()
    )
    .expect("string write");
    writeln!(
        output,
        "Engine-wide image candidates: {} managed image(s) ({})",
        image_candidates.len(),
        if result.images_requested {
            "explicitly selected with --images"
        } else {
            "not selected; add --images to acknowledge cross-repository scope"
        }
    )
    .expect("string write");
    for image in image_candidates {
        let name = image.tags.first().map_or(image.id.as_str(), String::as_str);
        writeln!(
            output,
            "  image {:>10}  {name}",
            format_bytes(image.cumulative_size_bytes)
        )
        .expect("string write");
    }
    for generation in state_candidates {
        writeln!(
            output,
            "  state {:>10}  {}",
            format_bytes(generation.logical_size_bytes),
            generation.path
        )
        .expect("string write");
    }
    if result.applied {
        writeln!(
            output,
            "Removed: {} image(s), {} managed-state path(s)",
            result.removed_images.len(),
            result.removed_state_generations.len()
        )
        .expect("string write");
        for failure in &result.failures {
            writeln!(output, "Failed: {} — {}", failure.target, failure.message)
                .expect("string write");
        }
    } else {
        writeln!(
            output,
            "No data was removed. Rerun with --apply to delete repository-local state; add --images only to include engine-wide image candidates."
        )
        .expect("string write");
    }
    writeln!(
        output,
        "Images are never deleted without both --apply and --images. The current image, legacy images, unclassified state, shared layers, and global build cache are retained."
    )
    .expect("string write");
    output
}

fn format_bytes(bytes: u64) -> String {
    if bytes == 0 {
        return String::from("0 B");
    }
    const UNITS: [(&str, u64); 4] = [
        ("GiB", 1024 * 1024 * 1024),
        ("MiB", 1024 * 1024),
        ("KiB", 1024),
        ("B", 1),
    ];
    let (unit, divisor) = UNITS
        .into_iter()
        .find(|(_, divisor)| bytes >= *divisor)
        .expect("byte unit");
    if divisor == 1 {
        format!("{bytes} B")
    } else {
        format!("{:.1} {unit}", bytes as f64 / divisor as f64)
    }
}

pub(crate) fn check(operation: CheckOperation, registry: &AdapterRegistry) -> ExitCode {
    managed_quality_result(
        "check",
        (|| {
            let (root, preparations, container_config) =
                prepared_quality_environment(&operation.config, registry, operation.authorization)?;
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
            ayni_environment::launch_repository_prepared(
                &root,
                &preparations,
                &command,
                launch_authorization(operation.authorization),
            )
        })(),
    )
}

pub(crate) fn verify(operation: VerifyOperation, registry: &AdapterRegistry) -> ExitCode {
    managed_quality_result(
        "verify",
        (|| {
            let (root, preparations, container_config) =
                prepared_quality_environment(&operation.config, registry, operation.authorization)?;
            let command = managed_verify_command(&operation, container_config);
            ayni_environment::launch_repository_prepared(
                &root,
                &preparations,
                &command,
                launch_authorization(operation.authorization),
            )
        })(),
    )
}

pub(crate) fn impact_run(operation: ImpactOperation, registry: &AdapterRegistry) -> ExitCode {
    managed_quality_result(
        "impact run",
        (|| {
            let (root, preparations, container_config) =
                prepared_quality_environment(&operation.config, registry, operation.authorization)?;
            let command = managed_impact_command(&operation, container_config);
            ayni_environment::launch_repository_prepared(
                &root,
                &preparations,
                &command,
                launch_authorization(operation.authorization),
            )
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
    authorization: CapabilityAuthorization,
) -> Result<
    (std::path::PathBuf, Vec<DependencyPreparationPlan>, String),
    ayni_environment::BackendError,
> {
    let repo_root = config
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let (root, plan) = current_plan(repo_root, Some(config), registry)?;
    authorize_capabilities(plan.capabilities(), authorization)?;
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
        authorize_capabilities(plan.capabilities(), operation.authorization)?;
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
            operation.authorization,
        ),
        Err(error) => render_error(error),
    }
}

pub(crate) fn run(operation: EnvRunOperation, registry: &AdapterRegistry) -> ExitCode {
    match current_plan(&operation.repo_root, None, registry).and_then(|(root, plan)| {
        authorize_capabilities(plan.capabilities(), operation.authorization)?;
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
            operation.authorization,
        ),
        Err(error) => render_error(error),
    }
}

fn authorize_capabilities(
    capabilities: ayni_core::EnvironmentCapabilities,
    authorization: CapabilityAuthorization,
) -> Result<(), ayni_environment::BackendError> {
    if capabilities.network == NetworkAccess::Bridge && !authorization.allow_network {
        return Err(ayni_environment::BackendError::environment(String::from(
            "the locked repository requests bridge networking; rerun with --allow-network only after reviewing the repository trust boundary",
        )));
    }
    if capabilities.docker == DockerAccess::Socket && !authorization.allow_docker_socket {
        return Err(ayni_environment::BackendError::environment(String::from(
            "the locked repository requests host Docker-daemon access; rerun with --allow-docker-socket only for a trusted repository and daemon",
        )));
    }
    Ok(())
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
    authorization: CapabilityAuthorization,
) -> ExitCode {
    match ayni_environment::launch_prepared(
        repo_root,
        &target,
        command,
        shell,
        preparations,
        launch_authorization(authorization),
    ) {
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

const fn launch_authorization(
    authorization: CapabilityAuthorization,
) -> ayni_environment::LaunchAuthorization {
    ayni_environment::LaunchAuthorization {
        allow_network: authorization.allow_network,
        allow_docker_socket: authorization.allow_docker_socket,
    }
}

fn render_error(error: ayni_environment::BackendError) -> ExitCode {
    crate::application_error::render_error(error.into())
}

#[cfg(test)]
mod tests {
    use super::{
        authorize_capabilities, format_bytes, managed_impact_command, managed_verify_command,
        render_storage_prune, render_storage_report,
    };
    use crate::application::{
        CapabilityAuthorization, ExecutionMode, ImpactOperation, OutputFormat, VerifyOperation,
    };
    use ayni_core::{DockerAccess, EnvironmentCapabilities, Language, NetworkAccess, SignalKind};
    use ayni_environment::{
        StorageImage, StorageImageOwnership, StorageImagePruneScope, StoragePruneResult,
        StorageReport, StorageStateGeneration,
    };
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
            authorization: CapabilityAuthorization::default(),
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
            authorization: CapabilityAuthorization::default(),
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

    #[test]
    fn elevated_capabilities_require_independent_operator_authorization() {
        let requested = EnvironmentCapabilities {
            docker: DockerAccess::Socket,
            network: NetworkAccess::Bridge,
        };
        let network_error = authorize_capabilities(
            requested,
            CapabilityAuthorization {
                allow_network: false,
                allow_docker_socket: true,
            },
        )
        .expect_err("network must be authorized");
        assert!(network_error.message.contains("--allow-network"));

        let socket_error = authorize_capabilities(
            requested,
            CapabilityAuthorization {
                allow_network: true,
                allow_docker_socket: false,
            },
        )
        .expect_err("socket must be authorized");
        assert!(socket_error.message.contains("--allow-docker-socket"));

        authorize_capabilities(
            requested,
            CapabilityAuthorization {
                allow_network: true,
                allow_docker_socket: true,
            },
        )
        .expect("explicit authorization");
    }

    #[test]
    fn storage_human_output_distinguishes_cumulative_size_and_safe_candidates() {
        let report = storage_fixture();
        let rendered = render_storage_report(&report);
        assert!(rendered.contains("Expected image tag: ayni-env:current"));
        assert!(rendered.contains("Current image present: yes"));
        assert!(rendered.contains("Images: 2 (3.0 KiB cumulative)"));
        assert!(rendered.contains("State root: 768 B total logical data"));
        assert!(rendered.contains("Classified environment state: 1 path(s), 512 B"));
        assert!(rendered.contains(
            "Unclassified state: 256 B (included in state-root total; reported, never pruned)"
        ));
        assert!(rendered.contains("current"));
        assert!(rendered.contains("stale"));
        assert!(rendered.contains("shared layers and engine build cache are not attributed"));
        assert!(rendered.contains("engine-wide across repositories"));
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1024), "1.0 KiB");
    }

    #[test]
    fn storage_human_output_does_not_claim_an_absent_current_image() {
        let mut report = storage_fixture();
        report.current_image_present = false;
        report.images.clear();
        report.image_cumulative_size_bytes = 0;

        let rendered = render_storage_report(&report);

        assert!(rendered.contains("Expected image tag: ayni-env:current"));
        assert!(rendered.contains("Current image present: no; run `ayni env build` to create it"));
        assert!(!rendered.contains("Current image: ayni-env:current"));
    }

    #[test]
    fn prune_human_output_keeps_dry_run_explicit() {
        let result = StoragePruneResult {
            applied: false,
            images_requested: false,
            report: storage_fixture(),
            removed_images: Vec::new(),
            removed_state_generations: Vec::new(),
            failures: Vec::new(),
        };
        let rendered = render_storage_prune(&result);
        assert!(rendered.contains("dry run"));
        assert!(rendered.contains("No data was removed"));
        assert!(rendered.contains("Rerun with --apply"));
        assert!(rendered.contains("not selected; add --images"));
    }

    fn storage_fixture() -> StorageReport {
        StorageReport {
            engine: String::from("docker"),
            expected_image_tag: String::from("ayni-env:current"),
            current_image_present: true,
            images: vec![
                StorageImage {
                    id: String::from("sha256:current"),
                    tags: vec![String::from("ayni-env:current")],
                    cumulative_size_bytes: 1024,
                    lock_fingerprint: Some(String::from("sha256:current")),
                    preparation_digest: Some(String::from("sha256:current")),
                    schema_version: Some(String::from("0.5.0")),
                    ownership: StorageImageOwnership::Managed,
                    current: true,
                    prune_candidate: false,
                },
                StorageImage {
                    id: String::from("sha256:stale"),
                    tags: vec![String::from("ayni-env:stale")],
                    cumulative_size_bytes: 2048,
                    lock_fingerprint: Some(String::from("sha256:stale")),
                    preparation_digest: Some(String::from("sha256:stale")),
                    schema_version: Some(String::from("0.5.0")),
                    ownership: StorageImageOwnership::Managed,
                    current: false,
                    prune_candidate: true,
                },
            ],
            image_cumulative_size_bytes: 3072,
            image_prune_scope: StorageImagePruneScope::EngineWideAcrossRepositories,
            state_root: String::from(".ayni/environment"),
            state_generations: vec![StorageStateGeneration {
                path: String::from(".ayni/environment/aaaaaaaaaaaaaaaa/bbbbbbbbbbbbbbbb"),
                logical_size_bytes: 512,
                current: false,
                prune_candidate: true,
            }],
            state_root_logical_size_bytes: 768,
            classified_state_logical_size_bytes: 512,
            unclassified_state_logical_size_bytes: 256,
            build_cache_included: false,
        }
    }
}
