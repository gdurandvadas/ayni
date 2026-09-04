use crate::analysis::{
    build_analyze_targets, enabled_signal_kinds, invalidate_artifact_at, managed_execution_active,
    persist_artifact_at, signal_kind_slug, workspace_root_from_config_path,
};
use crate::application::{ExecutionMode, ImpactOperation, OutputFormat};
use crate::build_registry;
use crate::policy::load_from_path;
use crate::ui::cancellation::SignalCancellation;
use ayni_adapters_common::exec::run_command_structured_cancellable;
use ayni_adapters_common::paths::validate_configured_root_containment;
use ayni_core::{
    AYNI_SIGNAL_SCHEMA_VERSION, AdapterRegistry, AyniPolicy, CancellationToken, ChangedPath,
    Findings, IMPACT_SCHEMA_VERSION, ImpactArtifact, ImpactConfidence, ImpactExecution,
    ImpactExecutionIssue, ImpactIdentity, ImpactIdentityKind, ImpactPlan, ImpactReason,
    ImpactReasonKind, ImpactRequest, ImpactUncertainty, ImpactUncertaintyKind, RunOutcome,
    SelectedCheck, SignalRow, VerificationSelection,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

const IMPACT_ARTIFACT: &str = ".ayni/impact/last/impact.json";
const GIT_TIMEOUT: Duration = Duration::from_secs(60);

mod git;
mod render;
use git::{GitSnapshot, git_snapshot};
use render::{effective_execution_mode, emit_artifact, emit_plan, execution_mode_name};

type Error = crate::application_error::ApplicationError;

pub(crate) fn show(operation: ImpactOperation) -> ExitCode {
    let registry = build_registry();
    let cancellation = match SignalCancellation::install() {
        Ok(cancellation) => cancellation,
        Err(error) => return fail(Error::execution(error)),
    };
    match prepare_plan(&operation, &registry, &cancellation.token()) {
        Ok((_, _, plan, _)) => match emit_plan(&plan, &operation) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => fail(error),
        },
        Err(error) => fail(error),
    }
}

pub(crate) fn run(operation: ImpactOperation) -> ExitCode {
    if let Err(error) = invalidate_run_artifact(&operation) {
        return fail(Error::execution(error));
    }
    if operation.managed_handoff.is_some() {
        return run_managed_handoff(operation);
    }
    let registry = build_registry();
    let cancellation = match SignalCancellation::install() {
        Ok(cancellation) => cancellation,
        Err(error) => return fail(Error::execution(error)),
    };
    match run_inner(&operation, &registry, &cancellation.token()) {
        Ok((_, true)) => ExitCode::from(4),
        Ok((true, false)) => ExitCode::from(1),
        Ok((false, false)) => ExitCode::SUCCESS,
        Err(error) => fail(error),
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct ManagedImpactHandoff {
    plan: ImpactPlan,
    source_fingerprint: String,
    config_path: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ManagedImpactDocument {
    schema_version: String,
    signal_schema_version: String,
    generated_at: String,
    execution_mode: String,
    plan: ImpactPlan,
    execution: ImpactExecution,
    repository_completion: serde_json::Value,
    aggregate: serde_json::Value,
    rows: Vec<SignalRow>,
    findings: Vec<Findings>,
}

pub(crate) fn invalidate_run_artifact(operation: &ImpactOperation) -> Result<(), String> {
    let workspace_root = workspace_root_from_config_path(&operation.config)?;
    invalidate_artifact_at(&workspace_root, IMPACT_ARTIFACT)
}

pub(crate) struct ManagedImpactSession {
    handoff: tempfile::NamedTempFile,
    result: tempfile::NamedTempFile,
    result_relative: String,
}

impl ManagedImpactSession {
    pub(crate) fn handoff_path(&self) -> &Path {
        self.handoff.path()
    }

    pub(crate) fn result_relative(&self) -> &str {
        &self.result_relative
    }
}

fn managed_result_directory(workspace_root: &Path) -> Result<PathBuf, String> {
    let mut directory = workspace_root.to_path_buf();
    for component in [".ayni", "impact", "pending"] {
        directory.push(component);
        match std::fs::symlink_metadata(&directory) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
            Ok(_) => {
                return Err(format!(
                    "managed impact result path {} must be a real directory",
                    directory.display()
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                std::fs::create_dir(&directory).map_err(|error| {
                    format!(
                        "failed to create managed impact result directory {}: {error}",
                        directory.display()
                    )
                })?;
            }
            Err(error) => {
                return Err(format!(
                    "failed to inspect managed impact result directory {}: {error}",
                    directory.display()
                ));
            }
        }
    }
    Ok(directory)
}

/// Create the immutable host-side plan and private provisional result slot used
/// by a managed inner execution.
pub(crate) fn prepare_managed_handoff(
    operation: &ImpactOperation,
    registry: &AdapterRegistry,
    prepared_outputs: &[String],
    managed_config_path: &str,
) -> Result<ManagedImpactSession, String> {
    let cancellation = CancellationToken::default();
    let (workspace_root, _, plan, _) =
        prepare_plan(operation, registry, &cancellation).map_err(|error| error.message)?;
    let source_fingerprint =
        crate::analysis::source_fingerprint_excluding(&workspace_root, prepared_outputs)?;
    let mut handoff = tempfile::NamedTempFile::new()
        .map_err(|error| format!("failed to create managed impact handoff: {error}"))?;
    serde_json::to_writer(
        &mut handoff,
        &ManagedImpactHandoff {
            plan,
            source_fingerprint,
            config_path: managed_config_path.to_owned(),
        },
    )
    .map_err(|error| format!("failed to serialize managed impact handoff: {error}"))?;
    handoff
        .flush()
        .map_err(|error| format!("failed to flush managed impact handoff: {error}"))?;

    let pending = managed_result_directory(&workspace_root)?;
    let result = tempfile::Builder::new()
        .prefix("impact-")
        .suffix(".json")
        .tempfile_in(&pending)
        .map_err(|error| format!("failed to create managed impact result: {error}"))?;
    let result_relative = result
        .path()
        .strip_prefix(&workspace_root)
        .map_err(|_| String::from("managed impact result escaped the repository"))?
        .to_string_lossy()
        .replace('\\', "/");
    Ok(ManagedImpactSession {
        handoff,
        result,
        result_relative,
    })
}

fn read_managed_handoff(session: &ManagedImpactSession) -> Result<ManagedImpactHandoff, String> {
    let bytes = std::fs::read(session.handoff.path())
        .map_err(|error| format!("failed to read managed impact handoff: {error}"))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("failed to parse managed impact handoff: {error}"))
}

/// Recompute the host candidate after managed execution so concurrent checkout
/// edits remain a fail-closed condition even though the container has no Git.
pub(crate) fn managed_handoff_is_stable(
    operation: &ImpactOperation,
    registry: &AdapterRegistry,
    session: &ManagedImpactSession,
) -> Result<bool, String> {
    let handoff = read_managed_handoff(session)?;
    let cancellation = CancellationToken::default();
    let (_, _, plan, _) =
        prepare_plan(operation, registry, &cancellation).map_err(|error| error.message)?;
    Ok(plan == handoff.plan)
}

pub(crate) fn promote_managed_result(
    operation: &ImpactOperation,
    session: &ManagedImpactSession,
    exit_code: i32,
    captured_stdout: &[u8],
) -> Result<(), String> {
    let serialized = std::fs::read_to_string(session.result.path())
        .map_err(|error| format!("managed impact did not produce provisional evidence: {error}"))?;
    let handoff = read_managed_handoff(session)?;
    let expected = validate_managed_document(&serialized, &handoff)?;
    validate_managed_findings(&expected, Path::new(&handoff.config_path))?;
    validate_managed_result_transport(
        operation.output,
        &serialized,
        &expected,
        exit_code,
        captured_stdout,
    )?;
    let workspace_root = workspace_root_from_config_path(&operation.config)?;
    persist_artifact_at(&workspace_root, IMPACT_ARTIFACT, &serialized)
}

fn validate_managed_document(
    serialized: &str,
    handoff: &ManagedImpactHandoff,
) -> Result<ImpactArtifact, String> {
    let document: serde_json::Value = serde_json::from_str(serialized).map_err(|error| {
        format!("managed impact produced invalid provisional evidence: {error}")
    })?;
    let parsed: ManagedImpactDocument =
        serde_json::from_value(document.clone()).map_err(|error| {
            format!("managed impact evidence does not match the artifact schema: {error}")
        })?;
    validate_managed_document_identity(&parsed, handoff)?;
    parsed.plan.validate().map_err(|error| error.to_string())?;
    for row in &parsed.rows {
        row.validate_payloads()?;
    }
    validate_managed_rows(&parsed.plan, &parsed.rows)?;
    validate_managed_issues(&parsed.plan, &parsed.execution)?;
    let expected = ImpactArtifact::new(
        parsed.generated_at,
        parsed.execution_mode,
        parsed.plan,
        parsed.execution.issues,
        parsed.rows,
        parsed.findings,
    );
    let expected_document = serde_json::to_value(&expected)
        .map_err(|error| format!("failed to validate managed impact evidence: {error}"))?;
    let _reported_derived_views = (parsed.repository_completion, parsed.aggregate);
    if document != expected_document {
        return Err(String::from(
            "managed impact provisional evidence has inconsistent execution or aggregate accounting",
        ));
    }
    Ok(expected)
}

fn validate_managed_document_identity(
    document: &ManagedImpactDocument,
    handoff: &ManagedImpactHandoff,
) -> Result<(), String> {
    let valid = document.schema_version == IMPACT_SCHEMA_VERSION
        && document.signal_schema_version == AYNI_SIGNAL_SCHEMA_VERSION
        && document.execution_mode == "managed"
        && document.plan == handoff.plan;
    if valid {
        Ok(())
    } else {
        Err(String::from(
            "managed impact provisional evidence does not match its immutable handoff",
        ))
    }
}

fn validate_managed_issues(plan: &ImpactPlan, execution: &ImpactExecution) -> Result<(), String> {
    let invalid = execution.issues.iter().any(|issue| {
        issue
            .check
            .as_ref()
            .is_some_and(|check| !plan.selected_checks.contains(check))
    });
    if invalid {
        Err(String::from(
            "managed impact evidence contains an issue for an unplanned check",
        ))
    } else {
        Ok(())
    }
}

fn validate_managed_rows(plan: &ImpactPlan, rows: &[SignalRow]) -> Result<(), String> {
    let mut matched = vec![false; plan.selected_checks.len()];
    for row in rows {
        let Some(index) = plan
            .selected_checks
            .iter()
            .enumerate()
            .position(|(index, check)| {
                !matched[index] && signal_row_matches(check, row, "/workspace")
            })
        else {
            return Err(String::from(
                "managed impact evidence contains a duplicate or unplanned signal row",
            ));
        };
        matched[index] = true;
    }
    Ok(())
}

fn validate_managed_findings(artifact: &ImpactArtifact, config_path: &Path) -> Result<(), String> {
    let registry = build_registry();
    let expected = materialize_findings(&artifact.rows, &registry, config_path, false)
        .map_err(|error| error.message)?;
    if expected == artifact.findings {
        Ok(())
    } else {
        Err(String::from(
            "managed impact findings do not match their signal-row offenders",
        ))
    }
}

fn validate_managed_result_transport(
    output: OutputFormat,
    serialized: &str,
    artifact: &ImpactArtifact,
    exit_code: i32,
    captured_stdout: &[u8],
) -> Result<(), String> {
    let expected_exit_code = impact_exit_code(artifact.outcome());
    if exit_code != expected_exit_code {
        return Err(format!(
            "managed impact exited with {exit_code}, but its provisional evidence requires exit code {expected_exit_code}"
        ));
    }
    if output == OutputFormat::Json && captured_stdout != serialized.as_bytes() {
        return Err(String::from(
            "managed impact JSON output does not match its provisional evidence",
        ));
    }
    Ok(())
}

fn impact_exit_code(outcome: RunOutcome) -> i32 {
    match outcome {
        RunOutcome::Passed => 0,
        RunOutcome::QualityFailed => 1,
        RunOutcome::ExecutionIncomplete => 4,
    }
}

fn managed_result_relative_path(operation: &ImpactOperation) -> Result<String, Error> {
    let path = operation.managed_result.as_ref().ok_or_else(|| {
        Error::input("managed impact execution requires a provisional result path")
    })?;
    if path.is_absolute() {
        return Err(Error::input(
            "managed impact result must be a repository-relative path",
        ));
    }
    let components = path
        .components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    let valid = components.len() == 4
        && components[0] == ".ayni"
        && components[1] == "impact"
        && components[2] == "pending"
        && components[3].starts_with("impact-")
        && components[3].ends_with(".json");
    if !valid {
        return Err(Error::input(
            "managed impact result must be a normalized private impact path",
        ));
    }
    Ok(components.join("/"))
}

struct ManagedHandoffContext {
    result_path: String,
    handoff: ManagedImpactHandoff,
    workspace_root: PathBuf,
    policy: AyniPolicy,
}

fn run_managed_handoff(operation: ImpactOperation) -> ExitCode {
    match run_managed_handoff_inner(&operation) {
        Ok(outcome) => ExitCode::from(impact_exit_code(outcome) as u8),
        Err(error) => fail(error),
    }
}

fn run_managed_handoff_inner(operation: &ImpactOperation) -> Result<RunOutcome, Error> {
    let context = load_managed_handoff_context(operation)?;
    let cancellation = CancellationToken::default();
    let before =
        crate::analysis::source_fingerprint(&context.workspace_root).map_err(Error::execution)?;
    if before != context.handoff.source_fingerprint {
        return Err(Error::execution(
            "managed workspace snapshot does not match the immutable host candidate",
        ));
    }
    let registry = build_registry();
    let mut collected = execute_checks(
        &context.workspace_root,
        &context.policy,
        &context.handoff.plan,
        &registry,
        operation,
        &cancellation,
    )?;
    record_managed_candidate_drift(&context.workspace_root, &before, &mut collected)?;
    let findings = materialize_findings(&collected.rows, &registry, &operation.config, false)?;
    let artifact = build_artifact(context.handoff.plan, collected, findings, operation)?;
    let serialized = serialize_impact_artifact(&artifact)?;
    ensure_not_cancelled(&cancellation, "impact execution")?;
    persist_artifact_at(&context.workspace_root, &context.result_path, &serialized)
        .map_err(Error::execution)?;
    emit_artifact(&artifact, &serialized, operation.output)?;
    Ok(artifact.outcome())
}

fn load_managed_handoff_context(
    operation: &ImpactOperation,
) -> Result<ManagedHandoffContext, Error> {
    validate_managed_handoff_invocation(operation)?;
    let result_path = managed_result_relative_path(operation)?;
    let path = operation
        .managed_handoff
        .as_ref()
        .expect("validated managed handoff path");
    let bytes = std::fs::read(path)
        .map_err(|error| Error::input(format!("failed to read managed impact handoff: {error}")))?;
    let handoff: ManagedImpactHandoff = serde_json::from_slice(&bytes).map_err(|error| {
        Error::input(format!("failed to parse managed impact handoff: {error}"))
    })?;
    if operation.config != Path::new(&handoff.config_path) {
        return Err(Error::input(
            "managed impact contract does not match its immutable handoff",
        ));
    }
    handoff
        .plan
        .validate()
        .map_err(|error| Error::input(error.to_string()))?;
    let workspace_root =
        workspace_root_from_config_path(&operation.config).map_err(Error::input)?;
    let config = operation
        .config
        .canonicalize()
        .map_err(|error| Error::input(format!("failed to resolve impact contract: {error}")))?;
    let policy = load_from_path(&config).map_err(Error::input)?;
    validate_configured_root_containment(&workspace_root, &policy).map_err(Error::input)?;
    Ok(ManagedHandoffContext {
        result_path,
        handoff,
        workspace_root,
        policy,
    })
}

fn validate_managed_handoff_invocation(operation: &ImpactOperation) -> Result<(), Error> {
    if std::env::var_os("AYNI_MANAGED_LOCK_FINGERPRINT").is_none() {
        return Err(Error::input(
            "--managed-handoff is reserved for managed environment execution",
        ));
    }
    let path = operation
        .managed_handoff
        .as_ref()
        .expect("managed handoff selected");
    if path != Path::new("/opt/ayni/inputs/impact-plan.json") {
        return Err(Error::input(
            "managed impact handoff must use the reserved read-only input path",
        ));
    }
    Ok(())
}

fn record_managed_candidate_drift(
    workspace_root: &Path,
    before: &str,
    collected: &mut CollectedImpact,
) -> Result<(), Error> {
    let after = crate::analysis::source_fingerprint(workspace_root).map_err(Error::execution)?;
    if before != after {
        collected.issues.push(candidate_drift_issue());
    }
    Ok(())
}

fn fail(error: Error) -> ExitCode {
    crate::application_error::render_error(error)
}

fn prepare_plan(
    operation: &ImpactOperation,
    registry: &AdapterRegistry,
    cancellation: &CancellationToken,
) -> Result<(PathBuf, AyniPolicy, ImpactPlan, GitSnapshot), Error> {
    let workspace_root =
        workspace_root_from_config_path(&operation.config).map_err(Error::input)?;
    ensure_impact_artifact_ignored(&workspace_root, cancellation)?;
    let config = operation
        .config
        .canonicalize()
        .map_err(|error| Error::input(format!("failed to resolve impact contract: {error}")))?;
    if !config.starts_with(&workspace_root) {
        return Err(Error::input("impact contract escapes the repository root"));
    }
    let policy = load_from_path(&config).map_err(Error::input)?;
    validate_configured_root_containment(&workspace_root, &policy).map_err(Error::input)?;
    let snapshot = git_snapshot(&workspace_root, &operation.base, cancellation)?;
    ensure_not_cancelled(cancellation, "impact planning")?;
    let config_path = config
        .strip_prefix(&workspace_root)
        .map_err(|_| Error::input("impact contract escapes the repository root"))?
        .to_string_lossy()
        .replace('\\', "/");
    let plan = plan_changes(
        &workspace_root,
        &policy,
        &snapshot,
        if config_path.is_empty() {
            ".ayni.toml"
        } else {
            &config_path
        },
        registry,
    )?;
    ensure_not_cancelled(cancellation, "impact planning")?;
    Ok((workspace_root, policy, plan, snapshot))
}

fn plan_changes(
    workspace_root: &Path,
    policy: &AyniPolicy,
    snapshot: &GitSnapshot,
    config_path: &str,
    registry: &AdapterRegistry,
) -> Result<ImpactPlan, Error> {
    let signals = enabled_signal_kinds(policy)
        .into_iter()
        .collect::<BTreeSet<_>>();
    let contract_changed = snapshot
        .changes
        .iter()
        .any(|change| change_touches(change, config_path));
    let environment_changed = snapshot
        .changes
        .iter()
        .any(|change| change_touches(change, ".ayni.lock"));
    let enabled = policy.enabled_languages().map_err(Error::input)?;
    let mut selected_checks = Vec::new();
    let mut uncertainties = Vec::new();

    for language in enabled {
        let adapter = registry
            .adapters()
            .iter()
            .find(|adapter| adapter.language() == language)
            .ok_or_else(|| Error::execution(format!("{language} adapter is unavailable")))?;
        for configured_root in policy.roots_for(language) {
            if contract_changed || environment_changed {
                for signal in &signals {
                    let (kind, detail) = if contract_changed {
                        (
                            ImpactReasonKind::ContractChanged,
                            format!("{config_path} changed and invalidates governed checks"),
                        )
                    } else {
                        (
                            ImpactReasonKind::EnvironmentChanged,
                            String::from(".ayni.lock changed and invalidates runtime checks"),
                        )
                    };
                    selected_checks.push(SelectedCheck::root(
                        language,
                        configured_root.clone(),
                        *signal,
                        ImpactReason { kind, detail },
                        ImpactConfidence::Certain,
                    ));
                }
                continue;
            }

            let request = ImpactRequest::new(
                workspace_root.to_path_buf(),
                language,
                configured_root.clone(),
                snapshot.changes.clone(),
                signals.iter().copied(),
            )
            .map_err(|error| Error::input(error.to_string()))?;
            match adapter.analyze_impact(&request) {
                Ok(contribution) => {
                    selected_checks.extend(contribution.selected_checks);
                    uncertainties.extend(contribution.uncertainties);
                }
                Err(error) => {
                    uncertainties.push(ImpactUncertainty {
                        kind: ImpactUncertaintyKind::Unsupported,
                        detail: format!(
                            "{}:{} impact mapping broadened: {}",
                            language, configured_root, error.message
                        ),
                    });
                    for signal in &signals {
                        selected_checks.push(SelectedCheck::root(
                            language,
                            configured_root.clone(),
                            *signal,
                            ImpactReason {
                                kind: ImpactReasonKind::UnsupportedCapability,
                                detail: String::from(
                                    "unsupported impact mapping requires configured-root execution",
                                ),
                            },
                            ImpactConfidence::Unknown,
                        ));
                    }
                }
            }
        }
    }

    let mut plan = ImpactPlan {
        base: ImpactIdentity {
            kind: ImpactIdentityKind::Revision,
            revision: snapshot.base_commit.clone(),
            requested: Some(snapshot.requested_base.clone()),
            fingerprint: None,
        },
        candidate: ImpactIdentity {
            kind: ImpactIdentityKind::WorkingTree,
            revision: snapshot.head_commit.clone(),
            requested: None,
            fingerprint: Some(snapshot.fingerprint.clone()),
        },
        changes: snapshot.changes.clone(),
        selected_checks,
        uncertainties,
        repository_completion_required: true,
    };
    plan.normalize();
    plan.validate()
        .map_err(|error| Error::input(error.to_string()))?;
    Ok(plan)
}

fn change_touches(change: &ChangedPath, path: &str) -> bool {
    change.path == path || change.previous_path.as_deref() == Some(path)
}

fn ensure_impact_artifact_ignored(
    workspace_root: &Path,
    cancellation: &CancellationToken,
) -> Result<(), Error> {
    let args = vec![
        String::from("check-ignore"),
        String::from("-q"),
        String::from("--"),
        String::from(IMPACT_ARTIFACT),
    ];
    let output =
        run_command_structured_cancellable(workspace_root, "git", &args, GIT_TIMEOUT, cancellation)
            .map_err(|error| Error::execution(error.to_string()))?;
    if output.status.success() {
        return Ok(());
    }
    if output.status.code() == Some(1) {
        return Err(Error::input(
            ".ayni/ must be ignored before impact planning can persist evidence",
        ));
    }
    Err(Error::input(format!(
        "git check-ignore failed: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    )))
}

fn run_inner(
    operation: &ImpactOperation,
    registry: &AdapterRegistry,
    cancellation: &CancellationToken,
) -> Result<(bool, bool), Error> {
    let execution = execute_impact_candidate(operation, registry, cancellation)?;
    let artifact = build_artifact(
        execution.plan,
        execution.collected,
        execution.findings,
        operation,
    )?;
    let serialized = serialize_impact_artifact(&artifact)?;
    persist_impact_artifact(
        &execution.workspace_root,
        &artifact,
        &serialized,
        operation,
        cancellation,
    )?;
    Ok(impact_outcome_flags(artifact.outcome()))
}

struct ImpactCandidateExecution {
    workspace_root: PathBuf,
    plan: ImpactPlan,
    collected: CollectedImpact,
    findings: Vec<Findings>,
}

fn execute_impact_candidate(
    operation: &ImpactOperation,
    registry: &AdapterRegistry,
    cancellation: &CancellationToken,
) -> Result<ImpactCandidateExecution, Error> {
    let (workspace_root, policy, plan, before) = prepare_plan(operation, registry, cancellation)?;
    let mut collected = execute_checks(
        &workspace_root,
        &policy,
        &plan,
        registry,
        operation,
        cancellation,
    )?;
    ensure_not_cancelled(cancellation, "impact execution")?;
    let findings = materialize_findings(
        &collected.rows,
        registry,
        &operation.config,
        !managed_execution_active(),
    )?;
    let (_, _, recomputed_plan, after) = prepare_plan(operation, registry, cancellation)?;
    ensure_not_cancelled(cancellation, "impact execution")?;
    if after != before || recomputed_plan != plan {
        collected.issues.push(candidate_drift_issue());
    }
    Ok(ImpactCandidateExecution {
        workspace_root,
        plan,
        collected,
        findings,
    })
}

fn serialize_impact_artifact(artifact: &ImpactArtifact) -> Result<String, Error> {
    serde_json::to_string_pretty(artifact)
        .map(|value| format!("{value}\n"))
        .map_err(|error| Error::execution(format!("failed to serialize impact artifact: {error}")))
}

fn persist_impact_artifact(
    workspace_root: &Path,
    artifact: &ImpactArtifact,
    serialized: &str,
    operation: &ImpactOperation,
    cancellation: &CancellationToken,
) -> Result<(), Error> {
    ensure_not_cancelled(cancellation, "impact execution")?;
    persist_artifact_at(workspace_root, IMPACT_ARTIFACT, serialized).map_err(Error::execution)?;
    emit_artifact(artifact, serialized, operation.output)?;
    Ok(())
}

fn impact_outcome_flags(outcome: RunOutcome) -> (bool, bool) {
    match outcome {
        RunOutcome::Passed => (false, false),
        RunOutcome::QualityFailed => (true, false),
        RunOutcome::ExecutionIncomplete => (false, true),
    }
}

struct CollectedImpact {
    rows: Vec<SignalRow>,
    executed_checks: Vec<SelectedCheck>,
    issues: Vec<ImpactExecutionIssue>,
}

fn execute_checks(
    workspace_root: &Path,
    policy: &AyniPolicy,
    plan: &ImpactPlan,
    registry: &AdapterRegistry,
    operation: &ImpactOperation,
    cancellation: &CancellationToken,
) -> Result<CollectedImpact, Error> {
    let planning = build_analyze_targets(
        workspace_root,
        policy,
        None,
        None,
        None,
        operation.debug,
        registry,
    )
    .map_err(Error::input)?;
    let target_by_key = planning
        .targets
        .iter()
        .map(|target| ((target.language, target.root.clone()), target))
        .collect::<BTreeMap<_, _>>();
    // Preflight each selected impact context, rather than its configured-root
    // base context: collectors can require different executables for package or
    // file-scoped work.
    let mut host_contexts = Vec::new();
    let mut host_check_specs = Vec::new();
    for check in &plan.selected_checks {
        let Some(base_target) = target_by_key.get(&(check.language, check.configured_root.clone()))
        else {
            continue;
        };
        let Some(adapter) = registry
            .adapters()
            .iter()
            .find(|adapter| adapter.language() == check.language)
        else {
            continue;
        };
        let mut context = base_target.run_context.clone();
        context.scope.package = check.package.clone();
        context.scope.file = check.file.clone();
        host_contexts.push(context);
        host_check_specs.push((check.language, check.signal, adapter.collector()));
    }
    let host_checks = host_contexts.iter().zip(host_check_specs).map(
        |(context, (language, signal, collector))| crate::host_prerequisites::SelectedCheck {
            language,
            signal,
            context,
            collector,
        },
    );
    crate::host_prerequisites::validate(workspace_root, policy, host_checks)
        .map_err(Error::execution)?;
    let mut collected = CollectedImpact {
        rows: Vec::new(),
        executed_checks: Vec::new(),
        issues: Vec::new(),
    };
    for check in &plan.selected_checks {
        ensure_not_cancelled(cancellation, "impact execution")?;
        let Some(base_target) = target_by_key.get(&(check.language, check.configured_root.clone()))
        else {
            collected.issues.push(ImpactExecutionIssue {
                check: Some(check.clone()),
                message: String::from("configured impact target was not detected or resolved"),
            });
            continue;
        };
        let adapter = registry
            .adapters()
            .iter()
            .find(|adapter| adapter.language() == check.language)
            .ok_or_else(|| Error::execution(format!("{} adapter unavailable", check.language)))?;
        let mut target = (*base_target).clone();
        target.run_context.cancellation = cancellation.clone();
        target.run_context.scope.package = check.package.clone();
        target.run_context.scope.file = check.file.clone();
        let selection = VerificationSelection {
            file: check.file.clone(),
            package: check.package.clone(),
            name: None,
        };
        log_check(check, "running");
        let row = match adapter.collect_verification(
            check.signal,
            &target.run_context,
            &selection,
            &mut |line| log_check(check, line),
        ) {
            Ok(row) => row,
            Err(error) => {
                collected.issues.push(ImpactExecutionIssue {
                    check: Some(check.clone()),
                    message: format!("collection incomplete: {error}"),
                });
                continue;
            }
        };
        if let Err(message) =
            reconcile_signal_row(check, &row, &target.run_context.scope.workspace_root)
        {
            collected.issues.push(ImpactExecutionIssue {
                check: Some(check.clone()),
                message,
            });
            continue;
        }
        if let Some(failure) = row.result.command_failure() {
            collected.issues.push(ImpactExecutionIssue {
                check: Some(check.clone()),
                message: format!(
                    "{} failed to execute: {}",
                    signal_kind_slug(check.signal),
                    failure.message
                ),
            });
        }
        collected.executed_checks.push(check.clone());
        collected.rows.push(row);
    }
    Ok(collected)
}

fn ensure_not_cancelled(cancellation: &CancellationToken, operation: &str) -> Result<(), Error> {
    if cancellation.is_cancelled() {
        Err(Error::execution(format!("{operation} aborted by Ctrl-C")))
    } else {
        Ok(())
    }
}

fn signal_row_matches(
    check: &SelectedCheck,
    row: &SignalRow,
    expected_workspace_root: &str,
) -> bool {
    let expected_path = (check.configured_root != ".").then_some(check.configured_root.as_str());
    row.language == check.language
        && row.kind == check.signal
        && row.scope.workspace_root == expected_workspace_root
        && row.scope.path.as_deref() == expected_path
        && row.scope.package == check.package
        && row.scope.file == check.file
}

fn reconcile_signal_row(
    check: &SelectedCheck,
    row: &SignalRow,
    expected_workspace_root: &str,
) -> Result<(), String> {
    if signal_row_matches(check, row, expected_workspace_root) {
        return Ok(());
    }
    let actual_path = row.scope.path.as_deref();
    Err(format!(
        "impact execution returned a row that does not match its selected check (expected language={}, signal={}, workspace_root={}, root={}, package={:?}, file={:?}; got language={}, signal={}, workspace_root={}, root={:?}, package={:?}, file={:?})",
        check.language,
        signal_kind_slug(check.signal),
        expected_workspace_root,
        check.configured_root,
        check.package,
        check.file,
        row.language,
        signal_kind_slug(row.kind),
        row.scope.workspace_root,
        actual_path,
        row.scope.package,
        row.scope.file,
    ))
}

fn log_check(check: &SelectedCheck, message: &str) {
    eprintln!(
        "[impact {}:{}] {} {message}",
        check.language,
        check.configured_root,
        signal_kind_slug(check.signal)
    );
}

fn candidate_drift_issue() -> ImpactExecutionIssue {
    ImpactExecutionIssue {
        check: None,
        message: String::from(
            "impact candidate changed during execution; rerun impact against a stable checkout",
        ),
    }
}

fn build_artifact(
    plan: ImpactPlan,
    collected: CollectedImpact,
    findings: Vec<Findings>,
    operation: &ImpactOperation,
) -> Result<ImpactArtifact, Error> {
    let generated_at = time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .map_err(|error| Error::execution(format!("failed to format timestamp: {error}")))?;
    Ok(ImpactArtifact::new(
        generated_at,
        execution_mode_name(effective_execution_mode(operation.execution_mode)),
        plan,
        collected.issues,
        collected.rows,
        findings,
    ))
}

fn materialize_findings(
    rows: &[SignalRow],
    registry: &AdapterRegistry,
    config_path: &Path,
    host_execution: bool,
) -> Result<Vec<Findings>, Error> {
    let mut result = Vec::with_capacity(rows.len());
    for row in rows {
        let adapter = registry
            .adapters()
            .iter()
            .find(|adapter| adapter.language() == row.language)
            .ok_or_else(|| Error::execution(format!("{} adapter unavailable", row.language)))?;
        let mut findings = adapter
            .findings_for(row, &row.scope.workspace_root)
            .map_err(|error| Error::execution(format!("failed to map impact findings: {error}")))?;
        let configured_root = row.scope.path.as_deref().unwrap_or(".");
        findings
            .render_commands(|target| {
                adapter
                    .verification_selector_support(row.kind)
                    .validate_target(row.kind, target)?;
                Ok(crate::verification_command::render_verification_command(
                    &config_path.to_string_lossy(),
                    row.kind,
                    row.language,
                    configured_root,
                    target,
                    host_execution,
                ))
            })
            .map_err(|error| Error::execution(error.to_string()))?;
        result.push(findings);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::git::{hash_untracked_path, parse_name_status};
    use super::render::effective_execution_mode_when;
    use super::{
        ExecutionMode, ManagedImpactHandoff, OutputFormat, reconcile_signal_row,
        validate_managed_document, validate_managed_findings, validate_managed_result_transport,
        validate_managed_rows,
    };
    use ayni_core::{
        Budget, CancellationToken, ChangeKind, ImpactArtifact, ImpactIdentity, ImpactIdentityKind,
        ImpactPlan, Language, Offenders, Scope, SelectedCheck, SignalKind, SignalResult, SignalRow,
        TestBudget, TestResult, lower_hex,
    };
    use sha2::{Digest, Sha256};

    #[test]
    fn managed_inner_execution_reports_managed_mode() {
        assert_eq!(
            effective_execution_mode_when(ExecutionMode::Host, true),
            ExecutionMode::Managed
        );
    }

    fn empty_managed_plan() -> ImpactPlan {
        ImpactPlan {
            base: ImpactIdentity {
                kind: ImpactIdentityKind::Revision,
                revision: String::from("base-commit"),
                requested: Some(String::from("main")),
                fingerprint: None,
            },
            candidate: ImpactIdentity {
                kind: ImpactIdentityKind::WorkingTree,
                revision: String::from("candidate-commit"),
                requested: None,
                fingerprint: Some(String::from("sha256:candidate")),
            },
            changes: Vec::new(),
            selected_checks: Vec::new(),
            uncertainties: Vec::new(),
            repository_completion_required: true,
        }
    }

    #[test]
    fn managed_result_validation_rejects_derived_accounting_and_transport_drift() {
        let plan = empty_managed_plan();
        let handoff = ManagedImpactHandoff {
            plan: plan.clone(),
            source_fingerprint: String::from("sha256:source"),
            config_path: String::from("./.ayni.toml"),
        };
        let artifact = ImpactArtifact::new(
            String::from("2026-09-01T00:00:00Z"),
            "managed",
            plan,
            Vec::new(),
            Vec::new(),
            Vec::new(),
        );
        let serialized = format!(
            "{}\n",
            serde_json::to_string_pretty(&artifact).expect("artifact serialization")
        );
        let validated = validate_managed_document(&serialized, &handoff).expect("valid evidence");
        assert!(
            validate_managed_result_transport(
                OutputFormat::Json,
                &serialized,
                &validated,
                0,
                serialized.as_bytes(),
            )
            .is_ok()
        );
        assert!(
            validate_managed_result_transport(
                OutputFormat::Json,
                &serialized,
                &validated,
                137,
                serialized.as_bytes(),
            )
            .is_err()
        );

        let mut inconsistent: serde_json::Value =
            serde_json::from_str(&serialized).expect("artifact JSON");
        inconsistent["aggregate"]["passing_rows"] = serde_json::json!(1);
        assert!(
            validate_managed_document(
                &serde_json::to_string_pretty(&inconsistent).expect("inconsistent JSON"),
                &handoff,
            )
            .is_err()
        );
    }

    #[test]
    fn parses_nul_delimited_changes_and_renames() {
        let changes = parse_name_status(b"M\0src/a.rs\0R100\0old.rs\0new.rs\0").expect("changes");
        assert_eq!(changes.len(), 2);
        assert_eq!(changes[0].kind, ChangeKind::Modified);
        assert_eq!(changes[0].path, "src/a.rs");
        assert_eq!(changes[1].kind, ChangeKind::Renamed);
        assert_eq!(changes[1].previous_path.as_deref(), Some("old.rs"));
        assert_eq!(changes[1].path, "new.rs");
    }

    #[test]
    fn parses_tab_separated_name_status_variant() {
        let changes = parse_name_status(b"A\tpath with spaces.rs\0").expect("changes");
        assert_eq!(changes[0].kind, ChangeKind::Added);
        assert_eq!(changes[0].path, "path with spaces.rs");
    }

    #[test]
    fn rejects_backslash_paths_instead_of_reinterpreting_them() {
        let error = parse_name_status(b"A\0path\\with\\backslashes.rs\0")
            .expect_err("backslashes are valid Unix filename bytes, not separators");
        assert!(error.message.contains("unsafe path"));
    }

    #[cfg(unix)]
    #[test]
    fn untracked_fingerprint_distinguishes_type_and_executable_mode() {
        use std::os::unix::fs::{PermissionsExt, symlink};

        let directory = tempfile::tempdir().expect("fixture");
        let path = directory.path().join("candidate");
        std::fs::write(&path, "target").expect("file");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))
            .expect("permissions");
        let regular = untracked_hash(directory.path(), "candidate");

        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .expect("permissions");
        let executable = untracked_hash(directory.path(), "candidate");
        assert_ne!(regular, executable);

        std::fs::remove_file(&path).expect("remove");
        symlink("target", &path).expect("symlink");
        let link = untracked_hash(directory.path(), "candidate");
        assert_ne!(regular, link);
    }

    #[cfg(unix)]
    fn untracked_hash(root: &std::path::Path, path: &str) -> String {
        let mut hasher = Sha256::new();
        hash_untracked_path(&mut hasher, root, path, &CancellationToken::default()).expect("hash");
        lower_hex(hasher.finalize())
    }
    #[test]
    fn rejects_mismatched_adapter_row_as_completed_impact_evidence() {
        let check = SelectedCheck::root(
            Language::Rust,
            String::from("crates/api"),
            SignalKind::Test,
            ayni_core::ImpactReason {
                kind: ayni_core::ImpactReasonKind::ChangedFile,
                detail: String::from("test"),
            },
            ayni_core::ImpactConfidence::High,
        );
        let row = SignalRow {
            kind: SignalKind::Test,
            language: Language::Node,
            scope: Scope {
                workspace_root: String::from("."),
                path: Some(String::from("crates/api")),
                package: None,
                file: Some(String::from("other.rs")),
            },
            pass: true,
            result: SignalResult::Test(TestResult {
                total_tests: 1,
                passed: 1,
                failed: 0,
                duration_ms: None,
                runner: String::from("test"),
                failure: None,
            }),
            budget: Budget::Test(TestBudget::default()),
            offenders: Offenders::Test(Vec::new()),
        };

        let mut plan = empty_managed_plan();
        plan.selected_checks.push(check.clone());
        assert!(validate_managed_rows(&plan, std::slice::from_ref(&row)).is_err());

        let mut matching_row = row.clone();
        matching_row.language = Language::Rust;
        matching_row.scope.workspace_root = String::from("/workspace");
        matching_row.scope.file = None;
        assert!(validate_managed_rows(&plan, &[matching_row.clone()]).is_ok());
        let missing_findings = ImpactArtifact::new(
            String::from("2026-09-01T00:00:00Z"),
            "managed",
            plan,
            Vec::new(),
            vec![matching_row],
            Vec::new(),
        );
        assert!(
            validate_managed_findings(&missing_findings, std::path::Path::new("./.ayni.toml"))
                .is_err()
        );

        let error = reconcile_signal_row(&check, &row, ".").expect_err("mismatched row");

        assert!(error.contains("does not match its selected check"));
        assert!(error.contains("language=rust"));
    }
}
