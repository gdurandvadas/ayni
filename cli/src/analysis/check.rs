use super::artifacts::ARTIFACTS_DIR;
use super::*;
use crate::build_registry;

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
    let config_path = PathBuf::from(config_path);
    let workspace_root = workspace_root_from_config_path(&config_path);
    let policy = policy::load_from_path(&config_path).map_err(AnalyzeError::InvalidContract)?;
    validate_configured_root_containment(&workspace_root, &policy)
        .map_err(AnalyzeError::InvalidContract)?;
    ensure_analyze_directories(&workspace_root)?;

    let output_mode = options.output_mode;
    let debug = options.debug;

    let registry = Arc::new(build_registry());
    let planning =
        build_analyze_targets(&workspace_root, &policy, None, None, None, debug, &registry)?;
    let plan = build_analyze_plan(&planning.targets);
    let metadata = build_artifact_metadata(&config_path, &workspace_root, &planning, output_mode)?;
    let artifact_slot = Arc::new(Mutex::new(None));
    let aborted = execute_analyze_plan_or_persist_failure(
        &workspace_root,
        &planning,
        &metadata,
        &options,
        plan,
        Arc::clone(&artifact_slot),
        Arc::clone(&registry),
    )?;
    if persist_aborted_analysis(&workspace_root, &planning, &metadata, aborted)? {
        return Err(AnalyzeError::Incomplete(String::from("check aborted")));
    }

    let mut artifact = take_collected_artifact_or_persist_failure(
        &workspace_root,
        &planning,
        &metadata,
        artifact_slot,
    )?;
    artifact.metadata = metadata;
    verification_command::materialize_finding_commands(
        &mut artifact,
        &registry,
        !managed_execution_active(),
    )?;
    let serialized = serialize_artifact(&artifact)?;
    persist_artifact_at(&workspace_root, SIGNALS_ARTIFACT, &serialized)?;
    emit_analyze_outputs(output_mode, &policy, &artifact, &serialized)?;

    Ok(artifact.outcome())
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
            eprintln!("[{language}] {name} started");
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
                "[{language}] {name} {state:?} {:.1}s",
                elapsed.as_secs_f64()
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
