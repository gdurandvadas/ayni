use super::artifacts::ARTIFACTS_DIR;
use super::*;
use crate::build_registry;
use std::time::{Duration, Instant};

pub(crate) fn analyze(
    config_path: &str,
    options: AnalyzeOptions,
) -> Result<RunOutcome, crate::application_error::ApplicationError> {
    analyze_impl(config_path, options).map_err(|error| match error {
        AnalyzeError::InvalidContract(message) => {
            crate::application_error::ApplicationError::input(message)
        }
        AnalyzeError::Incomplete(message) => {
            crate::application_error::ApplicationError::execution(message)
        }
    })
}

enum AnalyzeError {
    InvalidContract(String),
    Incomplete(String),
}

impl From<String> for AnalyzeError {
    fn from(error: String) -> Self {
        Self::Incomplete(error)
    }
}

fn analyze_impl(config_path: &str, options: AnalyzeOptions) -> Result<RunOutcome, AnalyzeError> {
    let total_started = Instant::now();
    let phase_started = Instant::now();
    let config_path = PathBuf::from(config_path);
    let (workspace_root, policy) = prepare_contract(&config_path)?;
    profile_phase(options.debug, "contract_setup", phase_started.elapsed());

    let output_mode = options.output_mode;
    let debug = options.debug;

    let phase_started = Instant::now();
    let registry = Arc::new(build_registry());
    let planning =
        build_analyze_targets(&workspace_root, &policy, None, None, None, debug, &registry)?;
    let plan = build_analyze_plan(&planning.targets);
    profile_phase(debug, "target_planning", phase_started.elapsed());
    let phase_started = Instant::now();
    let metadata =
        match build_artifact_metadata(&config_path, &workspace_root, &planning, output_mode) {
            Ok(metadata) => metadata,
            Err(error) => {
                invalidate_artifact_at(&workspace_root, SIGNALS_ARTIFACT)?;
                return Err(error.into());
            }
        };
    profile_phase(debug, "artifact_metadata", phase_started.elapsed());
    validate_host_prerequisites_or_persist(
        &workspace_root,
        &policy,
        &planning,
        &metadata,
        &registry,
    )?;
    let artifact_slot = Arc::new(Mutex::new(None));
    let phase_started = Instant::now();
    let aborted = execute_analyze_plan_or_persist_failure(
        &workspace_root,
        &planning,
        &metadata,
        &options,
        plan,
        Arc::clone(&artifact_slot),
        Arc::clone(&registry),
    )?;
    profile_phase(debug, "signal_collection", phase_started.elapsed());
    if persist_aborted_analysis(&workspace_root, &planning, &metadata, aborted)? {
        return Err(AnalyzeError::Incomplete(String::from("check aborted")));
    }

    let outcome = finalize_analysis(
        &workspace_root,
        &policy,
        &planning,
        &metadata,
        artifact_slot,
        &registry,
        output_mode,
        debug,
    )?;
    profile_phase(debug, "total", total_started.elapsed());
    Ok(outcome)
}

fn validate_host_prerequisites_or_persist(
    workspace_root: &Path,
    policy: &AyniPolicy,
    planning: &AnalyzePlanning,
    metadata: &RunArtifactMetadata,
    registry: &AdapterRegistry,
) -> Result<(), AnalyzeError> {
    let mut host_checks = Vec::new();
    for target in &planning.targets {
        let adapter = registry
            .adapters()
            .iter()
            .find(|adapter| adapter.language() == target.language)
            .expect("planned target adapter");
        let signals = enabled_signal_kinds(policy);
        let combines_test_and_coverage = signals.contains(&SignalKind::Test)
            && signals.contains(&SignalKind::Coverage)
            && adapter
                .collector()
                .supports_coverage_backed_test(&target.run_context);
        host_checks.extend(
            signals
                .into_iter()
                .filter(|signal| *signal != SignalKind::Test || !combines_test_and_coverage)
                .map(|signal| crate::host_prerequisites::SelectedCheck {
                    language: target.language,
                    signal,
                    context: &target.run_context,
                    collector: adapter.collector(),
                }),
        );
    }
    if let Err(error) = crate::host_prerequisites::validate(workspace_root, policy, host_checks) {
        persist_incomplete_execution_artifact(
            workspace_root,
            metadata.clone(),
            planning,
            CompletionStage::Resolution,
            &error,
        )?;
        return Err(error.into());
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn finalize_analysis(
    workspace_root: &Path,
    policy: &AyniPolicy,
    planning: &AnalyzePlanning,
    metadata: &RunArtifactMetadata,
    artifact_slot: Arc<Mutex<Option<RunArtifact>>>,
    registry: &AdapterRegistry,
    output_mode: OutputArg,
    debug: bool,
) -> Result<RunOutcome, AnalyzeError> {
    let phase_started = Instant::now();
    let mut artifact = take_collected_artifact_or_persist_failure(
        workspace_root,
        planning,
        metadata,
        artifact_slot,
    )?;
    artifact.metadata = metadata.clone();
    materialize_findings_or_persist_failure(
        workspace_root,
        planning,
        metadata,
        registry,
        &mut artifact,
    )?;
    profile_phase(debug, "finding_materialization", phase_started.elapsed());

    let phase_started = Instant::now();
    let serialized = serialize_or_persist_failure(workspace_root, planning, metadata, &artifact)?;
    persist_artifact_at(workspace_root, SIGNALS_ARTIFACT, &serialized)?;
    profile_phase(debug, "artifact_persistence", phase_started.elapsed());

    let phase_started = Instant::now();
    emit_analyze_outputs(output_mode, policy, &artifact, &serialized)?;
    profile_phase(debug, "output_rendering", phase_started.elapsed());
    Ok(artifact.outcome())
}

fn materialize_findings_or_persist_failure(
    workspace_root: &Path,
    planning: &AnalyzePlanning,
    metadata: &RunArtifactMetadata,
    registry: &AdapterRegistry,
    artifact: &mut RunArtifact,
) -> Result<(), AnalyzeError> {
    let result = verification_command::materialize_finding_commands(
        artifact,
        registry,
        !managed_execution_active(),
    );
    if let Err(error) = result {
        persist_incomplete_execution_artifact(
            workspace_root,
            metadata.clone(),
            planning,
            CompletionStage::Collection,
            &error,
        )?;
        return Err(error.into());
    }
    Ok(())
}

fn serialize_or_persist_failure(
    workspace_root: &Path,
    planning: &AnalyzePlanning,
    metadata: &RunArtifactMetadata,
    artifact: &RunArtifact,
) -> Result<String, AnalyzeError> {
    match serialize_artifact(artifact) {
        Ok(serialized) => Ok(serialized),
        Err(error) => {
            persist_incomplete_execution_artifact(
                workspace_root,
                metadata.clone(),
                planning,
                CompletionStage::Collection,
                &error,
            )?;
            Err(error.into())
        }
    }
}

fn prepare_contract(config_path: &Path) -> Result<(PathBuf, AyniPolicy), AnalyzeError> {
    let workspace_root =
        workspace_root_from_config_path(config_path).map_err(AnalyzeError::InvalidContract)?;
    // Tombstone the prior run before any current invocation work can fail.
    invalidate_artifact_at(&workspace_root, SIGNALS_ARTIFACT)?;
    let policy = policy::load_from_path(config_path).map_err(AnalyzeError::InvalidContract)?;
    validate_configured_root_containment(&workspace_root, &policy)
        .map_err(AnalyzeError::InvalidContract)?;
    ensure_analyze_directories(&workspace_root)?;
    Ok((workspace_root, policy))
}

fn profile_phase(debug: bool, phase: &str, elapsed: Duration) {
    if debug {
        eprintln!("[profile] phase={phase} elapsed_ms={}", elapsed.as_millis());
    }
}

fn execute_analyze_plan_or_persist_failure(
    workspace_root: &Path,
    planning: &AnalyzePlanning,
    metadata: &RunArtifactMetadata,
    options: &AnalyzeOptions,
    plan: ui::runner::Plan,
    artifact_slot: Arc<Mutex<Option<RunArtifact>>>,
    registry: Arc<AdapterRegistry>,
) -> Result<bool, String> {
    match execute_analyze_plan(
        options.output_mode,
        options.debug,
        plan,
        planning.clone(),
        artifact_slot,
        registry,
    ) {
        Ok(aborted) => Ok(aborted),
        Err(error) => {
            persist_incomplete_execution_artifact(
                workspace_root,
                metadata.clone(),
                planning,
                CompletionStage::Scheduling,
                &error,
            )?;
            Err(error)
        }
    }
}

fn persist_aborted_analysis(
    workspace_root: &Path,
    planning: &AnalyzePlanning,
    metadata: &RunArtifactMetadata,
    aborted: bool,
) -> Result<bool, String> {
    if aborted {
        persist_incomplete_execution_artifact(
            workspace_root,
            metadata.clone(),
            planning,
            CompletionStage::Collection,
            "analysis was interrupted before every target completed",
        )?;
    }
    Ok(aborted)
}

fn take_collected_artifact_or_persist_failure(
    workspace_root: &Path,
    planning: &AnalyzePlanning,
    metadata: &RunArtifactMetadata,
    artifact_slot: Arc<Mutex<Option<RunArtifact>>>,
) -> Result<RunArtifact, String> {
    match take_collected_artifact(artifact_slot) {
        Ok(artifact) => Ok(artifact),
        Err(error) => {
            persist_incomplete_execution_artifact(
                workspace_root,
                metadata.clone(),
                planning,
                CompletionStage::Collection,
                &error,
            )?;
            Err(error)
        }
    }
}

fn persist_incomplete_execution_artifact(
    workspace_root: &Path,
    metadata: RunArtifactMetadata,
    planning: &AnalyzePlanning,
    stage: CompletionStage,
    message: &str,
) -> Result<(), String> {
    let artifact = RunArtifact::new(
        metadata,
        planning.completion(
            CompletionScope::Repository,
            0,
            planning.runnable_failure_issues(stage, message),
        ),
        Vec::new(),
    )?;
    let serialized = serialize_artifact(&artifact)?;
    persist_artifact_at(workspace_root, SIGNALS_ARTIFACT, &serialized)
}

fn ensure_analyze_directories(workspace_root: &Path) -> Result<(), String> {
    fs::create_dir_all(workspace_root.join(ARTIFACTS_DIR)).map_err(|error| error.to_string())?;
    Ok(())
}

fn execute_analyze_plan(
    output_mode: OutputArg,
    debug: bool,
    plan: ui::runner::Plan,
    planning: AnalyzePlanning,
    artifact_slot: Arc<Mutex<Option<RunArtifact>>>,
    registry: Arc<AdapterRegistry>,
) -> Result<bool, String> {
    let execution = build_analyze_execution(planning, artifact_slot, registry);
    if debug {
        return ui::runner::run_plain(plan, execution, debug_progress_event)
            .map(|outcome| outcome.aborted);
    }
    match output_mode {
        OutputArg::Md | OutputArg::Json => {
            ui::runner::run_plain(plan, execution, ui::progress_log::log_started_check)
                .map(|outcome| outcome.aborted)
        }
        OutputArg::Stdout => run_stdout_plan(plan, execution),
    }
}

fn debug_progress_event(event: ui::runner::ProgressEvent) {
    match event {
        ui::runner::ProgressEvent::Started { language, name } => {
            eprintln!("[profile] collector_start language={language} signal={name}");
        }
        ui::runner::ProgressEvent::Line {
            language,
            name,
            line,
        } => {
            eprintln!("[{language}] {name}: {line}");
        }
        ui::runner::ProgressEvent::Finished {
            language,
            name,
            state,
            elapsed,
        } => {
            eprintln!(
                "[profile] collector_finish language={language} signal={name} state={state:?} elapsed_ms={}",
                elapsed.as_millis()
            );
        }
    }
}

fn build_analyze_execution(
    planning: AnalyzePlanning,
    artifact_slot: Arc<Mutex<Option<RunArtifact>>>,
    registry: Arc<AdapterRegistry>,
) -> impl FnOnce(ui::runner::ExecContext) -> Result<(), String> {
    move |exec_ctx: ui::runner::ExecContext| {
        let artifact =
            run_collect_with_ui(&exec_ctx, &planning, CompletionScope::Repository, registry)?;
        let mut slot = artifact_slot
            .lock()
            .map_err(|_| String::from("artifact mutex poisoned"))?;
        *slot = Some(artifact);
        Ok(())
    }
}

fn run_stdout_plan(
    plan: ui::runner::Plan,
    execution: impl FnOnce(ui::runner::ExecContext) -> Result<(), String> + Send + 'static,
) -> Result<bool, String> {
    if ui::is_interactive_stdout() {
        ui::runner::run(plan, execution).map(|outcome| outcome.aborted)
    } else {
        ui::fallback::run(&plan, execution)?;
        Ok(false)
    }
}

fn take_collected_artifact(
    artifact_slot: Arc<Mutex<Option<RunArtifact>>>,
) -> Result<RunArtifact, String> {
    let artifact = artifact_slot
        .lock()
        .map_err(|_| String::from("artifact mutex poisoned"))?
        .take();
    artifact.ok_or_else(|| String::from("analyze produced no artifact"))
}
