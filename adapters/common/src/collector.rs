//! Common boundary between collector failures and typed signal rows.

use crate::exec::ExecutionError;
use crate::failure::command_failure_from_execution_error;
use ayni_core::{
    AdapterError, Budget, ComplexityBudget, ComplexityResult, CoverageBudget, CoverageResult,
    DepsBudget, DepsResult, Language, MutationBudget, MutationResult, Offenders, RunContext,
    SignalKind, SignalResult, SignalRow, SizeBudget, SizeResult, TestBudget, TestResult,
};

/// A failure returned while collecting one signal.
///
/// Runner failures are structurally distinct from adapter failures so the
/// dispatch boundary can emit a completed, typed failed row only when command
/// execution itself could not complete. Tool non-zero exits remain normal
/// command output and are classified by their owning collector.
#[derive(Debug)]
pub enum CollectorError {
    /// A failure owned by the shared command runner.
    Execution(Box<ExecutionError>),
    /// A language adapter parsing, report, or configuration failure.
    Adapter(String),
}

impl From<Box<ExecutionError>> for CollectorError {
    fn from(error: Box<ExecutionError>) -> Self {
        Self::Execution(error)
    }
}

/// Internal result type for collectors that opt into structured runner errors.
pub type CollectorResult = Result<SignalRow, CollectorError>;

/// Internal result for a single execution that emits both test and coverage evidence.
pub type CoverageBackedTestResult = Result<(SignalRow, SignalRow), CollectorError>;

/// Finishes coverage-backed collection at an adapter dispatch boundary.
/// Runner failures are projected into both typed rows because neither signal
/// received complete evidence from the shared execution.
pub fn finish_coverage_backed_test(
    language: Language,
    context: &RunContext,
    result: CoverageBackedTestResult,
) -> Result<(SignalRow, SignalRow), AdapterError> {
    match result {
        Ok((mut test, mut coverage)) => {
            enforce_coverage_backed_evidence(context, &mut test, &mut coverage);
            Ok((test, coverage))
        }
        Err(CollectorError::Execution(error)) => Ok((
            execution_error_row(language, SignalKind::Test, context, (*error).clone()),
            execution_error_row(language, SignalKind::Coverage, context, *error),
        )),
        Err(CollectorError::Adapter(message)) => Err(AdapterError::new(language, message)),
    }
}

fn enforce_coverage_backed_evidence(
    context: &RunContext,
    test_row: &mut SignalRow,
    coverage_row: &mut SignalRow,
) {
    let coverage_complete = matches!(
        &coverage_row.result,
        SignalResult::Coverage(result)
            if result.status == "ok"
                && result.failure.is_none()
                && result.headline_percent().is_some_and(f64::is_finite)
    );
    if !coverage_complete
        && test_row.pass
        && let SignalResult::Test(result) = &mut test_row.result
    {
        test_row.pass = false;
        result.failure = Some(ayni_core::CommandFailure {
            category: String::from("repo_setup_issue"),
            classification: String::from("incomplete_combined_evidence"),
            command: result.runner.clone(),
            cwd: context.execution.exec_cwd.display().to_string(),
            exit_code: None,
            message: String::from(
                "coverage-backed execution did not produce complete coverage evidence; test evidence was rejected",
            ),
        });
    }
    if !test_row.pass
        && coverage_row.pass
        && let SignalResult::Coverage(result) = &mut coverage_row.result
    {
        coverage_row.pass = false;
        result.status = String::from("error");
        result.failure = Some(ayni_core::CommandFailure {
            category: String::from("repo_code_issue"),
            classification: String::from("incomplete_combined_evidence"),
            command: result.engine.clone(),
            cwd: context.execution.exec_cwd.display().to_string(),
            exit_code: None,
            message: String::from(
                "coverage-backed execution did not produce passing, complete test evidence",
            ),
        });
    }
}

/// Finishes collection at an adapter dispatch boundary.
///
/// Execution failures become a typed, failed row for the requested signal,
/// preserving the requested scope. Other collector failures remain adapter
/// errors, so they cannot be mistaken for completed collection evidence.
pub fn finish_collection(
    language: Language,
    kind: SignalKind,
    context: &RunContext,
    result: CollectorResult,
) -> Result<SignalRow, AdapterError> {
    match result {
        Ok(row) => Ok(row),
        Err(CollectorError::Execution(error)) => {
            Ok(execution_error_row(language, kind, context, *error))
        }
        Err(CollectorError::Adapter(message)) => Err(AdapterError::new(language, message)),
    }
}

fn execution_error_row(
    language: Language,
    kind: SignalKind,
    context: &RunContext,
    error: ExecutionError,
) -> SignalRow {
    let failure = command_failure_from_execution_error(kind, &error);
    let scope = context.scope.clone();
    match kind {
        SignalKind::Test => SignalRow {
            kind,
            language,
            scope,
            pass: false,
            result: SignalResult::Test(TestResult {
                total_tests: 0,
                passed: 0,
                failed: 0,
                duration_ms: None,
                runner: error.command,
                failure: Some(failure),
            }),
            budget: Budget::Test(TestBudget::default()),
            offenders: Offenders::Test(Vec::new()),
        },
        SignalKind::Coverage => SignalRow {
            kind,
            language,
            scope,
            pass: false,
            result: SignalResult::Coverage(CoverageResult {
                percent: None,
                line_percent: None,
                branch_percent: None,
                engine: error.command,
                status: String::from("error"),
                failure: Some(failure),
            }),
            budget: Budget::Coverage(CoverageBudget::default()),
            offenders: Offenders::Coverage(Vec::new()),
        },
        SignalKind::Size => SignalRow {
            kind,
            language,
            scope,
            pass: false,
            result: SignalResult::Size(SizeResult {
                max_lines: 0,
                total_files: 0,
                warn_count: 0,
                fail_count: 1,
                failure: Some(failure),
            }),
            budget: Budget::Size(SizeBudget::default()),
            offenders: Offenders::Size(Vec::new()),
        },
        SignalKind::Complexity => SignalRow {
            kind,
            language,
            scope,
            pass: false,
            result: SignalResult::Complexity(ComplexityResult {
                engine: error.command,
                method: String::from("command"),
                measured_functions: 0,
                max_fn_cyclomatic: 0.0,
                max_fn_cognitive: None,
                warn_count: 0,
                fail_count: 1,
                failure: Some(failure),
            }),
            budget: Budget::Complexity(ComplexityBudget::default()),
            offenders: Offenders::Complexity(Vec::new()),
        },
        SignalKind::Deps => SignalRow {
            kind,
            language,
            scope,
            pass: false,
            result: SignalResult::Deps(DepsResult {
                crate_count: 0,
                edge_count: 0,
                violation_count: 1,
                failure: Some(failure),
            }),
            budget: Budget::Deps(DepsBudget::default()),
            offenders: Offenders::Deps(Vec::new()),
        },
        SignalKind::Mutation => SignalRow {
            kind,
            language,
            scope,
            pass: false,
            result: SignalResult::Mutation(MutationResult {
                engine: error.command,
                killed: 0,
                survived: 0,
                timeout: 0,
                score: None,
                failure: Some(failure),
            }),
            budget: Budget::Mutation(MutationBudget::default()),
            offenders: Offenders::Mutation(Vec::new()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{CollectorError, finish_collection};
    use crate::exec::{ExecutionError, ExecutionErrorKind};
    use ayni_core::{AyniPolicy, ExecutionResolution, Language, RunContext, Scope, SignalKind};
    use std::path::PathBuf;
    use std::time::Duration;

    fn context() -> RunContext {
        let root = PathBuf::from("workspace");
        RunContext {
            repo_root: root.clone(),
            target_root: root.clone(),
            workdir: root.clone(),
            policy: AyniPolicy::default(),
            scope: Scope {
                workspace_root: String::from("."),
                path: Some(String::from("member")),
                package: Some(String::from("package")),
                file: Some(String::from("src/lib.rs")),
            },
            execution: ExecutionResolution::direct("tool", root, "test", 100),
            cancellation: Default::default(),
            debug: false,
        }
    }

    fn execution_error() -> CollectorError {
        CollectorError::Execution(Box::new(ExecutionError {
            kind: ExecutionErrorKind::Timeout,
            command: String::from("tool check"),
            cwd: PathBuf::from("workspace"),
            status: None,
            stdout: Vec::new(),
            stderr: Vec::new(),
            stdout_truncated_bytes: 0,
            stderr_truncated_bytes: 0,
            timeout: Some(Duration::from_secs(12)),
            detail: String::new(),
        }))
    }

    #[test]
    fn execution_errors_become_typed_failed_rows() {
        let context = context();
        for kind in [
            SignalKind::Test,
            SignalKind::Coverage,
            SignalKind::Size,
            SignalKind::Complexity,
            SignalKind::Deps,
            SignalKind::Mutation,
        ] {
            let row = finish_collection(Language::Rust, kind, &context, Err(execution_error()))
                .expect("execution failures become rows");
            assert_eq!(row.kind, kind);
            assert_eq!(row.language, Language::Rust);
            assert_eq!(row.scope, context.scope);
            assert!(!row.pass);
            let failure = row.result.command_failure().expect("typed failure");
            assert_eq!(failure.classification, "timeout");
            assert_eq!(failure.command, "tool check");
        }
    }

    #[test]
    fn adapter_errors_remain_adapter_errors() {
        let error = finish_collection(
            Language::Rust,
            SignalKind::Test,
            &context(),
            Err(CollectorError::Adapter(String::from("bad report"))),
        )
        .expect_err("adapter errors must not become rows");
        assert_eq!(error.language, Language::Rust);
        assert_eq!(error.message, "bad report");
    }
}
