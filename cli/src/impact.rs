use crate::analysis::{
    build_analyze_targets, enabled_signal_kinds, managed_execution_active, persist_artifact_at,
    signal_kind_slug, workspace_root_from_config_path,
};
use crate::application::{ExecutionMode, ImpactOperation, OutputFormat};
use crate::build_registry;
use crate::policy::load_from_path;
use ayni_adapters_common::exec::run_command_structured;
use ayni_adapters_common::paths::validate_configured_root_containment;
use ayni_core::{
    AdapterRegistry, AyniPolicy, ChangedPath, Findings, IMPACT_SCHEMA_VERSION, ImpactArtifact,
    ImpactConfidence, ImpactExecutionIssue, ImpactIdentity, ImpactIdentityKind, ImpactPlan,
    ImpactReason, ImpactReasonKind, ImpactRequest, ImpactUncertainty, ImpactUncertaintyKind,
    RunOutcome, SelectedCheck, SignalRow, VerificationSelection,
};
use std::collections::{BTreeMap, BTreeSet};
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
    crate::application_error::render_error(error)
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
    let policy = load_from_path(&config).map_err(Error::input)?;
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
    Ok(match artifact.outcome() {
        RunOutcome::Passed => (false, false),
        RunOutcome::QualityFailed => (true, false),
        RunOutcome::ExecutionIncomplete => (false, true),
    })
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

fn reconcile_signal_row(
    check: &SelectedCheck,
    row: &SignalRow,
    expected_workspace_root: &str,
) -> Result<(), String> {
    let expected_path = (check.configured_root != ".").then_some(check.configured_root.as_str());
    let actual_path = row.scope.path.as_deref();
    if row.language == check.language
        && row.kind == check.signal
        && row.scope.workspace_root == expected_workspace_root
        && actual_path == expected_path
        && row.scope.package == check.package
        && row.scope.file == check.file
    {
        return Ok(());
    }
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

#[cfg(test)]
mod tests {
    use super::git::{hash_untracked_path, parse_name_status};
    use super::render::effective_execution_mode_when;
    use super::{ExecutionMode, reconcile_signal_row};
    use ayni_core::{
        Budget, ChangeKind, Language, Offenders, Scope, SelectedCheck, SignalKind, SignalResult,
        SignalRow, TestResult,
    };
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
            budget: Budget::Test(serde_json::json!({})),
            offenders: Offenders::Test(Vec::new()),
        };

        let error = reconcile_signal_row(&check, &row, ".").expect_err("mismatched row");

        assert!(error.contains("does not match its selected check"));
        assert!(error.contains("language=rust"));
    }
}
