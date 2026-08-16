use crate::application::{ExecutionMode, ImpactOperation, OutputFormat};
use crate::{
    build_analyze_targets, build_registry, enabled_signal_kinds, failed_signal_row,
    managed_execution_active, persist_artifact_at, signal_kind_slug,
    workspace_root_from_config_path,
};
use ayni_adapters_common::exec::run_command_structured;
use ayni_adapters_common::paths::validate_configured_root_containment;
use ayni_core::{
    AYNI_SIGNAL_SCHEMA_VERSION, AdapterRegistry, AyniPolicy, ChangeKind, ChangedPath, Findings,
    ImpactConfidence, ImpactIdentity, ImpactIdentityKind, ImpactPlan, ImpactReason,
    ImpactReasonKind, ImpactRequest, ImpactUncertainty, ImpactUncertaintyKind, SelectedCheck,
    SignalRow, VerificationSelection,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

const IMPACT_SCHEMA_VERSION: &str = "0.1.0";
const IMPACT_ARTIFACT: &str = ".ayni/impact/last/impact.json";
const GIT_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug)]
struct Error {
    code: u8,
    message: String,
}

impl Error {
    fn input(message: impl Into<String>) -> Self {
        Self {
            code: 2,
            message: message.into(),
        }
    }

    fn execution(message: impl Into<String>) -> Self {
        Self {
            code: 4,
            message: message.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct GitSnapshot {
    requested_base: String,
    base_commit: String,
    head_commit: String,
    fingerprint: String,
    changes: Vec<ChangedPath>,
}

#[derive(Serialize)]
struct PlanEnvelope<'a> {
    schema_version: &'static str,
    execution_mode: &'static str,
    plan: &'a ImpactPlan,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ImpactExecutionState {
    Complete,
    Incomplete,
}

#[derive(Debug, Serialize)]
struct ImpactExecutionIssue {
    #[serde(skip_serializing_if = "Option::is_none")]
    check: Option<SelectedCheck>,
    message: String,
}

#[derive(Debug, Serialize)]
struct ImpactExecution {
    state: ImpactExecutionState,
    planned_jobs: u64,
    completed_jobs: u64,
    skipped_jobs: u64,
    issues: Vec<ImpactExecutionIssue>,
}

#[derive(Debug, Serialize)]
struct RepositoryCompletionMarker {
    evaluated: bool,
    required_command: &'static str,
}

#[derive(Debug, Serialize)]
struct ImpactAggregate {
    status: &'static str,
    passing_rows: u64,
    failing_rows: u64,
    scope: &'static str,
}

#[derive(Debug, Serialize)]
struct ImpactArtifact {
    schema_version: &'static str,
    signal_schema_version: &'static str,
    generated_at: String,
    execution_mode: &'static str,
    plan: ImpactPlan,
    execution: ImpactExecution,
    repository_completion: RepositoryCompletionMarker,
    aggregate: ImpactAggregate,
    rows: Vec<SignalRow>,
    findings: Vec<Findings>,
}

pub(crate) fn show(operation: ImpactOperation) -> ExitCode {
    match prepare_plan(&operation) {
        Ok((_, _, plan, _)) => match emit_plan(&plan, &operation) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => fail(error),
        },
        Err(error) => fail(error),
    }
}

pub(crate) fn run(operation: ImpactOperation) -> ExitCode {
    match run_inner(&operation) {
        Ok((_, true)) => ExitCode::from(4),
        Ok((true, false)) => ExitCode::from(1),
        Ok((false, false)) => ExitCode::SUCCESS,
        Err(error) => fail(error),
    }
}

fn fail(error: Error) -> ExitCode {
    eprintln!("{}", error.message);
    ExitCode::from(error.code)
}

fn prepare_plan(
    operation: &ImpactOperation,
) -> Result<(PathBuf, AyniPolicy, ImpactPlan, GitSnapshot), Error> {
    let workspace_root = workspace_root_from_config_path(&operation.config)
        .canonicalize()
        .map_err(|error| Error::input(format!("failed to resolve repository root: {error}")))?;
    ensure_impact_artifact_ignored(&workspace_root)?;
    let config = operation
        .config
        .canonicalize()
        .map_err(|error| Error::input(format!("failed to resolve impact contract: {error}")))?;
    if !config.starts_with(&workspace_root) {
        return Err(Error::input("impact contract escapes the repository root"));
    }
    let policy = AyniPolicy::load_from_path(&config).map_err(Error::input)?;
    validate_configured_root_containment(&workspace_root, &policy).map_err(Error::input)?;
    let snapshot = git_snapshot(&workspace_root, &operation.base)?;
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
    )?;
    Ok((workspace_root, policy, plan, snapshot))
}

fn plan_changes(
    workspace_root: &Path,
    policy: &AyniPolicy,
    snapshot: &GitSnapshot,
    config_path: &str,
) -> Result<ImpactPlan, Error> {
    let registry = build_registry();
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

fn ensure_impact_artifact_ignored(workspace_root: &Path) -> Result<(), Error> {
    let args = vec![
        String::from("check-ignore"),
        String::from("-q"),
        String::from("--"),
        String::from(IMPACT_ARTIFACT),
    ];
    let output = run_command_structured(workspace_root, "git", &args, GIT_TIMEOUT)
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

fn run_inner(operation: &ImpactOperation) -> Result<(bool, bool), Error> {
    let (workspace_root, policy, plan, before) = prepare_plan(operation)?;
    let registry = build_registry();
    let mut collected = execute_checks(&workspace_root, &policy, &plan, &registry, operation)?;
    let findings = materialize_findings(&collected.rows, &registry, operation)?;
    let (_, _, recomputed_plan, after) = prepare_plan(operation)?;
    if after != before || recomputed_plan != plan {
        collected.issues.push(candidate_drift_issue());
    }
    let artifact = build_artifact(plan, collected, findings, operation)?;
    let serialized = serde_json::to_string_pretty(&artifact)
        .map(|value| format!("{value}\n"))
        .map_err(|error| {
            Error::execution(format!("failed to serialize impact artifact: {error}"))
        })?;
    persist_artifact_at(&workspace_root, IMPACT_ARTIFACT, &serialized).map_err(Error::execution)?;
    emit_artifact(&artifact, &serialized, operation.output)?;
    Ok((
        artifact.aggregate.failing_rows > 0,
        artifact.execution.state == ImpactExecutionState::Incomplete,
    ))
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
) -> Result<CollectedImpact, Error> {
    let planning = build_analyze_targets(workspace_root, policy, None, None, None, operation.debug)
        .map_err(Error::input)?;
    let target_by_key = planning
        .targets
        .iter()
        .map(|target| ((target.language, target.root.clone()), target))
        .collect::<BTreeMap<_, _>>();
    let mut collected = CollectedImpact {
        rows: Vec::new(),
        executed_checks: Vec::new(),
        issues: Vec::new(),
    };
    for check in &plan.selected_checks {
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
        target.run_context.scope.package = check.package.clone();
        target.run_context.scope.file = check.file.clone();
        let selection = VerificationSelection {
            file: check.file.clone(),
            package: check.package.clone(),
            name: None,
        };
        log_check(check, "running");
        let row = adapter
            .collect_verification(check.signal, &target.run_context, &selection, &mut |line| {
                log_check(check, line)
            })
            .unwrap_or_else(|error| {
                failed_signal_row(
                    check.language,
                    check.signal,
                    &target.run_context,
                    error.to_string(),
                )
            });
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
    let state = if collected.issues.is_empty() {
        ImpactExecutionState::Complete
    } else {
        ImpactExecutionState::Incomplete
    };
    let passing_rows = collected.rows.iter().filter(|row| row.pass).count() as u64;
    let failing_rows = collected.rows.len() as u64 - passing_rows;
    let planned_jobs = plan.selected_checks.len() as u64;
    let completed_jobs = collected.executed_checks.len() as u64;
    Ok(ImpactArtifact {
        schema_version: IMPACT_SCHEMA_VERSION,
        signal_schema_version: AYNI_SIGNAL_SCHEMA_VERSION,
        generated_at: time::OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Rfc3339)
            .map_err(|error| Error::execution(format!("failed to format timestamp: {error}")))?,
        execution_mode: execution_mode_name(effective_execution_mode(operation.execution_mode)),
        plan,
        repository_completion: RepositoryCompletionMarker {
            evaluated: false,
            required_command: "ayni check",
        },
        aggregate: ImpactAggregate {
            status: aggregate_status(state, failing_rows),
            passing_rows,
            failing_rows,
            scope: "selected_impact_plan_only",
        },
        execution: ImpactExecution {
            state,
            planned_jobs,
            completed_jobs,
            skipped_jobs: planned_jobs.saturating_sub(completed_jobs),
            issues: collected.issues,
        },
        rows: collected.rows,
        findings,
    })
}

fn aggregate_status(state: ImpactExecutionState, failing_rows: u64) -> &'static str {
    if state == ImpactExecutionState::Complete && failing_rows == 0 {
        "pass"
    } else {
        "fail"
    }
}

fn materialize_findings(
    rows: &[SignalRow],
    registry: &AdapterRegistry,
    operation: &ImpactOperation,
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
                    &operation.config.to_string_lossy(),
                    row.kind,
                    row.language,
                    configured_root,
                    target,
                    !managed_execution_active(),
                ))
            })
            .map_err(|error| Error::execution(error.to_string()))?;
        result.push(findings);
    }
    Ok(result)
}

fn emit_plan(plan: &ImpactPlan, operation: &ImpactOperation) -> Result<(), Error> {
    match operation.output {
        OutputFormat::Json => {
            let envelope = PlanEnvelope {
                schema_version: IMPACT_SCHEMA_VERSION,
                execution_mode: execution_mode_name(effective_execution_mode(
                    operation.execution_mode,
                )),
                plan,
            };
            let serialized = serde_json::to_string_pretty(&envelope).map_err(|error| {
                Error::execution(format!("failed to serialize impact plan: {error}"))
            })?;
            println!("{serialized}");
        }
        OutputFormat::Markdown => {
            print_plan_markdown(plan, effective_execution_mode(operation.execution_mode));
        }
        OutputFormat::Human => {
            print_plan_human(plan, effective_execution_mode(operation.execution_mode));
        }
    }
    Ok(())
}

fn emit_artifact(
    artifact: &ImpactArtifact,
    serialized: &str,
    output: OutputFormat,
) -> Result<(), Error> {
    match output {
        OutputFormat::Json => print!("{serialized}"),
        OutputFormat::Markdown => {
            print_plan_markdown(&artifact.plan, mode_from_name(artifact.execution_mode));
            println!("\n## Impact execution\n");
            println!(
                "- State: `{:?}`\n- Jobs: {}/{}\n- Selected rows passing: {}/{}",
                artifact.execution.state,
                artifact.execution.completed_jobs,
                artifact.execution.planned_jobs,
                artifact.aggregate.passing_rows,
                artifact.rows.len()
            );
        }
        OutputFormat::Human => {
            print_plan_human(&artifact.plan, mode_from_name(artifact.execution_mode));
            println!("execution: {:?}", artifact.execution.state);
            println!(
                "selected jobs: {}/{} completed; {} passing, {} failing",
                artifact.execution.completed_jobs,
                artifact.execution.planned_jobs,
                artifact.aggregate.passing_rows,
                artifact.aggregate.failing_rows
            );
            for issue in &artifact.execution.issues {
                println!("  issue: {}", issue.message);
            }
        }
    }
    Ok(())
}

fn print_plan_human(plan: &ImpactPlan, mode: ExecutionMode) {
    println!("ayni impact plan");
    println!(
        "base: {} -> {}",
        plan.base.requested.as_deref().unwrap_or("<unknown>"),
        plan.base.revision
    );
    println!(
        "candidate: working tree at {} ({})",
        plan.candidate.revision,
        plan.candidate.fingerprint.as_deref().unwrap_or("<unknown>")
    );
    println!("execution mode: {}", execution_mode_name(mode));
    println!("changes: {}", plan.changes.len());
    for change in &plan.changes {
        println!("  {:?} {}", change.kind, change.path);
    }
    println!("selected checks: {}", plan.selected_checks.len());
    for check in &plan.selected_checks {
        let scope = check
            .package
            .as_deref()
            .map(|value| format!("package {value}"))
            .or_else(|| check.file.as_deref().map(|value| format!("file {value}")))
            .unwrap_or_else(|| String::from("configured root"));
        println!(
            "  {}:{} {} ({scope}, {:?})",
            check.language,
            check.configured_root,
            signal_kind_slug(check.signal),
            check.confidence
        );
        for reason in &check.reasons {
            println!("    because {:?}: {}", reason.kind, reason.detail);
        }
    }
    for uncertainty in &plan.uncertainties {
        println!(
            "  uncertainty {:?}: {}",
            uncertainty.kind, uncertainty.detail
        );
    }
    println!("impact evidence is not repository completion; run `ayni check`");
}

fn print_plan_markdown(plan: &ImpactPlan, mode: ExecutionMode) {
    println!("# Ayni impact plan\n");
    println!(
        "> Impact evidence covers only the selected change. Run `ayni check` for repository completion.\n"
    );
    println!("- Base: `{}`", plan.base.revision);
    println!("- Candidate HEAD: `{}`", plan.candidate.revision);
    println!("- Execution mode: `{}`", execution_mode_name(mode));
    println!("- Changed paths: {}", plan.changes.len());
    println!("- Selected checks: {}\n", plan.selected_checks.len());
    for check in &plan.selected_checks {
        println!(
            "- `{}` `{}` `{}`",
            check.language,
            check.configured_root,
            signal_kind_slug(check.signal)
        );
        for reason in &check.reasons {
            println!("  - {:?}: {}", reason.kind, reason.detail);
        }
    }
}

fn effective_execution_mode(requested: ExecutionMode) -> ExecutionMode {
    effective_execution_mode_when(requested, managed_execution_active())
}

fn effective_execution_mode_when(requested: ExecutionMode, managed: bool) -> ExecutionMode {
    if managed {
        ExecutionMode::Managed
    } else {
        requested
    }
}

fn execution_mode_name(mode: ExecutionMode) -> &'static str {
    match mode {
        ExecutionMode::Managed => "managed",
        ExecutionMode::Host => "host",
    }
}

fn mode_from_name(name: &str) -> ExecutionMode {
    if name == "managed" {
        ExecutionMode::Managed
    } else {
        ExecutionMode::Host
    }
}

fn git_snapshot(workspace_root: &Path, requested_base: &str) -> Result<GitSnapshot, Error> {
    let context = resolve_git_context(workspace_root, requested_base)?;
    reject_conflicts(&context.root)?;
    let tracked = git_bytes(
        &context.root,
        &[
            "diff",
            "--name-status",
            "-z",
            "--find-renames",
            &context.base_commit,
            "--",
        ],
    )?;
    let untracked = nul_strings(&git_bytes(
        &context.root,
        &["ls-files", "--others", "--exclude-standard", "-z"],
    )?)?;
    let changes = collect_workspace_changes(&tracked, &untracked, &context.prefix)?;
    let fingerprint = candidate_fingerprint(&context, &untracked)?;
    Ok(GitSnapshot {
        requested_base: requested_base.to_owned(),
        base_commit: context.base_commit,
        head_commit: context.head_commit,
        fingerprint,
        changes,
    })
}

struct GitContext {
    root: PathBuf,
    prefix: String,
    base_commit: String,
    head_commit: String,
}

fn resolve_git_context(workspace_root: &Path, requested_base: &str) -> Result<GitContext, Error> {
    let root = PathBuf::from(git_text(workspace_root, &["rev-parse", "--show-toplevel"])?.trim())
        .canonicalize()
        .map_err(|error| Error::input(format!("failed to resolve Git root: {error}")))?;
    let prefix = workspace_root
        .strip_prefix(&root)
        .map_err(|_| Error::input("configured repository root is outside the Git worktree"))?
        .to_string_lossy()
        .replace('\\', "/");
    let base_arg = format!("{requested_base}^{{commit}}");
    let base_commit = git_text(
        &root,
        &["rev-parse", "--verify", "--end-of-options", &base_arg],
    )?
    .trim()
    .to_owned();
    let head_commit = git_text(
        &root,
        &["rev-parse", "--verify", "--end-of-options", "HEAD^{commit}"],
    )?
    .trim()
    .to_owned();
    Ok(GitContext {
        root,
        prefix,
        base_commit,
        head_commit,
    })
}

fn reject_conflicts(git_root: &Path) -> Result<(), Error> {
    let conflicts = git_bytes(
        git_root,
        &["diff", "--name-only", "--diff-filter=U", "-z", "--"],
    )?;
    if conflicts.is_empty() {
        Ok(())
    } else {
        Err(Error::input(
            "impact planning requires a working tree without unresolved Git conflicts",
        ))
    }
}

fn collect_workspace_changes(
    tracked: &[u8],
    untracked: &[String],
    prefix: &str,
) -> Result<Vec<ChangedPath>, Error> {
    let mut changes = Vec::new();
    for change in parse_name_status(tracked)? {
        changes.extend(change_for_workspace(change, prefix));
    }
    for path in untracked {
        let normalized = normalize_git_path(path)?;
        if let Some(path) = path_for_workspace(&normalized, prefix) {
            changes.push(ChangedPath {
                kind: ChangeKind::Added,
                path,
                previous_path: None,
            });
        }
    }
    changes.sort();
    changes.dedup();
    Ok(changes)
}

fn candidate_fingerprint(context: &GitContext, untracked: &[String]) -> Result<String, Error> {
    let binary_diff = git_bytes(
        &context.root,
        &["diff", "--binary", &context.base_commit, "--"],
    )?;
    let mut hasher = Sha256::new();
    hash_segment(&mut hasher, b"base", context.base_commit.as_bytes());
    hash_segment(&mut hasher, b"head", context.head_commit.as_bytes());
    hash_segment(&mut hasher, b"tracked_diff", &binary_diff);
    for path in untracked {
        hash_untracked_path(&mut hasher, &context.root, path)?;
    }
    Ok(format!("sha256:{:x}", hasher.finalize()))
}

fn hash_untracked_path(hasher: &mut Sha256, git_root: &Path, path: &str) -> Result<(), Error> {
    hash_segment(hasher, b"untracked_path", path.as_bytes());
    let candidate = git_root.join(path);
    let metadata = fs::symlink_metadata(&candidate).map_err(|error| {
        Error::input(format!("failed to inspect untracked path {path}: {error}"))
    })?;
    if metadata.file_type().is_symlink() {
        hash_segment(hasher, b"untracked_type", b"symlink");
        let target = fs::read_link(&candidate).map_err(|error| {
            Error::input(format!("failed to read untracked symlink {path}: {error}"))
        })?;
        hash_segment(
            hasher,
            b"untracked_symlink_target",
            target.as_os_str().as_encoded_bytes(),
        );
    } else if metadata.is_file() {
        hash_segment(hasher, b"untracked_type", b"file");
        hash_segment(
            hasher,
            b"untracked_executable",
            &[executable_bit(&metadata)],
        );
        let contents = fs::read(&candidate).map_err(|error| {
            Error::input(format!("failed to read untracked file {path}: {error}"))
        })?;
        hash_segment(hasher, b"untracked_contents", &contents);
    } else {
        return Err(Error::input(format!(
            "unsupported untracked filesystem entry {path}"
        )));
    }
    Ok(())
}

fn hash_segment(hasher: &mut Sha256, label: &[u8], value: &[u8]) {
    hasher.update((label.len() as u64).to_be_bytes());
    hasher.update(label);
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value);
}

#[cfg(unix)]
fn executable_bit(metadata: &fs::Metadata) -> u8 {
    use std::os::unix::fs::PermissionsExt;
    u8::from(metadata.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn executable_bit(_metadata: &fs::Metadata) -> u8 {
    0
}

fn git_text(workdir: &Path, args: &[&str]) -> Result<String, Error> {
    let bytes = git_bytes(workdir, args)?;
    String::from_utf8(bytes).map_err(|_| Error::input("Git returned a non-UTF-8 identity or path"))
}

fn git_bytes(workdir: &Path, args: &[&str]) -> Result<Vec<u8>, Error> {
    let args = args
        .iter()
        .map(|value| (*value).to_owned())
        .collect::<Vec<_>>();
    let output = run_command_structured(workdir, "git", &args, GIT_TIMEOUT)
        .map_err(|error| Error::execution(error.to_string()))?;
    if !output.status.success() {
        return Err(Error::input(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(output.stdout)
}

fn parse_name_status(bytes: &[u8]) -> Result<Vec<ChangedPath>, Error> {
    let tokens = nul_strings(bytes)?;
    let mut index = 0;
    let mut changes = Vec::new();
    while let Some(token) = tokens.get(index) {
        index += 1;
        let (status, first_path) = status_and_path(token, &tokens, &mut index)?;
        changes.push(parse_status(status, first_path, &tokens, &mut index)?);
    }
    Ok(changes)
}

fn status_and_path(
    token: &str,
    tokens: &[String],
    index: &mut usize,
) -> Result<(char, String), Error> {
    let (status, inline_path) = token
        .split_once('\t')
        .map_or((token, None), |(status, path)| (status, Some(path)));
    let code = status
        .chars()
        .next()
        .ok_or_else(|| Error::input("Git emitted an empty change status"))?;
    let path = if let Some(path) = inline_path {
        path.to_owned()
    } else {
        let path = tokens
            .get(*index)
            .ok_or_else(|| Error::input("Git change status is missing a path"))?
            .clone();
        *index += 1;
        path
    };
    Ok((code, path))
}

fn parse_status(
    code: char,
    first_path: String,
    tokens: &[String],
    index: &mut usize,
) -> Result<ChangedPath, Error> {
    match code {
        'R' | 'C' => {
            let destination = tokens
                .get(*index)
                .ok_or_else(|| Error::input("Git rename/copy status is missing a destination"))?;
            *index += 1;
            Ok(ChangedPath {
                kind: if code == 'R' {
                    ChangeKind::Renamed
                } else {
                    ChangeKind::Copied
                },
                path: normalize_git_path(destination)?,
                previous_path: Some(normalize_git_path(&first_path)?),
            })
        }
        'A' | 'M' | 'D' | 'T' => Ok(ChangedPath {
            kind: simple_change_kind(code),
            path: normalize_git_path(&first_path)?,
            previous_path: None,
        }),
        other => Err(Error::input(format!(
            "unsupported Git change status {other:?}; resolve the worktree state first"
        ))),
    }
}

fn simple_change_kind(code: char) -> ChangeKind {
    match code {
        'A' => ChangeKind::Added,
        'D' => ChangeKind::Deleted,
        'T' => ChangeKind::TypeChanged,
        _ => ChangeKind::Modified,
    }
}

fn nul_strings(bytes: &[u8]) -> Result<Vec<String>, Error> {
    bytes
        .split(|byte| *byte == 0)
        .filter(|token| !token.is_empty())
        .map(|token| {
            String::from_utf8(token.to_vec())
                .map_err(|_| Error::input("Git returned a non-UTF-8 repository path"))
        })
        .collect()
}

fn normalize_git_path(path: &str) -> Result<String, Error> {
    if path.is_empty()
        || path.contains('\\')
        || path.starts_with('/')
        || path.split('/').any(|part| part == "..")
    {
        return Err(Error::input(format!(
            "Git returned an unsafe path: {path:?}"
        )));
    }
    Ok(path.to_owned())
}

fn path_for_workspace(path: &str, prefix: &str) -> Option<String> {
    if prefix.is_empty() {
        return Some(path.to_owned());
    }
    path.strip_prefix(prefix)
        .and_then(|path| path.strip_prefix('/'))
        .map(str::to_owned)
}

fn change_for_workspace(change: ChangedPath, prefix: &str) -> Vec<ChangedPath> {
    let current = path_for_workspace(&change.path, prefix);
    let previous = change
        .previous_path
        .as_deref()
        .and_then(|path| path_for_workspace(path, prefix));
    match (change.kind, current, previous) {
        (ChangeKind::Renamed | ChangeKind::Copied, Some(path), Some(previous_path)) => {
            vec![ChangedPath {
                kind: change.kind,
                path,
                previous_path: Some(previous_path),
            }]
        }
        (ChangeKind::Renamed, Some(path), None) | (ChangeKind::Copied, Some(path), None) => {
            vec![ChangedPath {
                kind: ChangeKind::Added,
                path,
                previous_path: None,
            }]
        }
        (ChangeKind::Renamed, None, Some(path)) => vec![ChangedPath {
            kind: ChangeKind::Deleted,
            path,
            previous_path: None,
        }],
        (_, Some(path), _) => vec![ChangedPath {
            kind: change.kind,
            path,
            previous_path: None,
        }],
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ExecutionMode, effective_execution_mode_when, hash_untracked_path, parse_name_status,
    };
    use ayni_core::ChangeKind;
    use sha2::{Digest, Sha256};

    #[test]
    fn managed_inner_execution_reports_managed_mode() {
        assert_eq!(
            effective_execution_mode_when(ExecutionMode::Host, true),
            ExecutionMode::Managed
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
        hash_untracked_path(&mut hasher, root, path).expect("hash");
        format!("{:x}", hasher.finalize())
    }
}
