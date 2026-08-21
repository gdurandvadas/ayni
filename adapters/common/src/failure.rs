//! Shared command-failure classification and `CommandFailure` construction.

use crate::exec::{ExecutionError, ExecutionErrorKind, format_command};
use ayni_core::{CommandFailure, ConfiguredMetricEvaluation, RunContext, SignalKind};
use std::process::Output;

const FAILURE_DIAGNOSTIC_LIMIT: usize = 32 * 1024;
const FAILURE_DIAGNOSTIC_HEAD: usize = 8 * 1024;

/// Maps a signal kind to its documented failure category (see
/// `docs/product/runtime.md`).
pub fn failure_category(kind: SignalKind) -> &'static str {
    match kind {
        SignalKind::Test | SignalKind::Coverage | SignalKind::Mutation => "repo_code_issue",
        SignalKind::Complexity => "repo_setup_issue",
        SignalKind::Size | SignalKind::Deps => "ayni_internal_issue",
    }
}

/// stderr if non-empty, else stdout, else a placeholder message.
pub fn combined_output(output: &Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !stderr.is_empty() {
        stderr
    } else if !stdout.is_empty() {
        stdout
    } else {
        String::from("command failed without stdout/stderr output")
    }
}

/// First non-empty line across stderr then stdout.
pub fn concise_failure_message(output: &Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    format!("{stderr}\n{stdout}")
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| String::from("command failed without stdout/stderr output"))
}

/// Whether a non-zero test runner exit represents incomplete execution.
///
/// A parseable report with at least one failed test is complete quality
/// evidence: the command failed because the repository's tests failed. Missing
/// tests or a non-zero exit without reported failures instead indicates that
/// setup, collection, or execution did not complete.
#[must_use]
pub fn test_execution_incomplete(success: bool, total_tests: u64, failed_tests: u64) -> bool {
    !success && (total_tests == 0 || failed_tests == 0)
}

/// Builds a `CommandFailure` with the default `command_error` classification.
pub fn command_failure_from_output(
    context: &RunContext,
    kind: SignalKind,
    program: &str,
    args: &[String],
    output: &Output,
) -> CommandFailure {
    command_failure_with_classification(context, kind, program, args, output, "command_error")
}

/// Converts a runner-owned failure into the same serialized failure shape as a
/// tool's non-zero exit.  Unlike an `Output`, runner failures retain the exact
/// command, working directory, and configured timeout that caused the error.
pub fn command_failure_from_execution_error(
    kind: SignalKind,
    error: &ExecutionError,
) -> CommandFailure {
    let classification = match error.kind {
        ExecutionErrorKind::Spawn => "command_error",
        ExecutionErrorKind::Wait => "command_error",
        ExecutionErrorKind::Timeout => "timeout",
        ExecutionErrorKind::Cancelled => "cancelled",
        ExecutionErrorKind::OutputLimit => "output_limit",
    };
    let timeout_detail = error
        .timeout
        .map(|timeout| format!(" (configured timeout: {}s)", timeout.as_secs_f64()))
        .unwrap_or_default();
    let diagnostics = execution_diagnostics(error);
    CommandFailure {
        category: failure_category(kind).to_string(),
        classification: classification.to_string(),
        command: error.command.clone(),
        cwd: error.cwd.display().to_string(),
        exit_code: error.status.and_then(|status| status.code()),
        message: format!("{}{timeout_detail}{diagnostics}", error),
    }
}

fn execution_diagnostics(error: &ExecutionError) -> String {
    let stdout = diagnostic_excerpt(&error.stdout);
    let stderr = diagnostic_excerpt(&error.stderr);
    let captured = match (stdout.is_empty(), stderr.is_empty()) {
        (true, true) => String::new(),
        (false, true) => format!("\ncaptured stdout:\n{stdout}"),
        (true, false) => format!("\ncaptured stderr:\n{stderr}"),
        (false, false) => format!("\ncaptured stderr:\n{stderr}\ncaptured stdout:\n{stdout}"),
    };
    let truncation = match (error.stdout_truncated_bytes, error.stderr_truncated_bytes) {
        (0, 0) => String::new(),
        (stdout, 0) => format!("\nstdout truncated by at least {stdout} bytes"),
        (0, stderr) => format!("\nstderr truncated by at least {stderr} bytes"),
        (stdout, stderr) => format!(
            "\nstdout truncated by at least {stdout} bytes; stderr truncated by at least {stderr} bytes"
        ),
    };
    format!("{captured}{truncation}")
}

fn diagnostic_excerpt(bytes: &[u8]) -> String {
    if bytes.len() <= FAILURE_DIAGNOSTIC_LIMIT {
        return String::from_utf8_lossy(bytes).trim().to_string();
    }
    let tail = FAILURE_DIAGNOSTIC_LIMIT - FAILURE_DIAGNOSTIC_HEAD;
    let omitted = bytes.len() - FAILURE_DIAGNOSTIC_LIMIT;
    let head = String::from_utf8_lossy(&bytes[..FAILURE_DIAGNOSTIC_HEAD]);
    let tail = String::from_utf8_lossy(&bytes[bytes.len() - tail..]);
    format!(
        "{}\n[... {omitted} captured bytes omitted ...]\n{}",
        head.trim_start(),
        tail.trim_end()
    )
}

/// Builds a `CommandFailure` with an adapter-supplied classification and the
/// default concise message. Adapters that recognize tool-specific failure
/// modes (import errors, empty test sets, …) classify before calling this.
pub fn command_failure_with_classification(
    context: &RunContext,
    kind: SignalKind,
    program: &str,
    args: &[String],
    output: &Output,
    classification: &str,
) -> CommandFailure {
    CommandFailure {
        category: failure_category(kind).to_string(),
        classification: classification.to_string(),
        command: format_command(program, args),
        cwd: context.execution.exec_cwd.display().to_string(),
        exit_code: output.status.code(),
        message: concise_failure_message(output),
    }
}

/// Builds the `repo_setup_issue`/`missing_report` failure used when a tool
/// succeeded but its expected report file is absent.
pub fn setup_failure(
    context: &RunContext,
    command: String,
    message: impl Into<String>,
) -> CommandFailure {
    CommandFailure {
        category: String::from("repo_setup_issue"),
        classification: String::from("missing_report"),
        command,
        cwd: context.execution.exec_cwd.display().to_string(),
        exit_code: None,
        message: message.into(),
    }
}

/// Maps required coverage-metric evidence failures to stable setup failures.
///
/// The metric evaluation belongs to core; this helper only supplies the common
/// adapter failure representation. Finite and unconfigured metrics do not need
/// a command failure.
#[must_use]
pub fn coverage_metric_failure(
    context: &RunContext,
    command: String,
    metric: &str,
    evaluation: ConfiguredMetricEvaluation,
) -> Option<CommandFailure> {
    let (classification, message) = match evaluation {
        ConfiguredMetricEvaluation::Missing => (
            "missing_coverage_metric",
            format!(
                "coverage metric `{metric}` is missing; configure the coverage tool to emit it or remove its threshold"
            ),
        ),
        ConfiguredMetricEvaluation::Unparseable => (
            "unparseable_coverage_metric",
            format!(
                "coverage metric `{metric}` is not finite; configure the coverage tool to emit a finite percentage or remove its threshold"
            ),
        ),
        ConfiguredMetricEvaluation::Unconfigured | ConfiguredMetricEvaluation::Present { .. } => {
            return None;
        }
    };
    Some(CommandFailure {
        category: String::from("repo_setup_issue"),
        classification: classification.to_string(),
        command,
        cwd: context.execution.exec_cwd.display().to_string(),
        exit_code: None,
        message,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        combined_output, command_failure_from_execution_error, concise_failure_message,
        coverage_metric_failure, failure_category, test_execution_incomplete,
    };
    use crate::exec::{ExecutionError, ExecutionErrorKind};
    use ayni_core::{
        AyniPolicy, ConfiguredMetricEvaluation, ExecutionResolution, RunContext, Scope, SignalKind,
    };
    use std::os::unix::process::ExitStatusExt;
    use std::path::PathBuf;
    use std::process::{ExitStatus, Output};
    use std::time::Duration;

    fn output(stdout: &str, stderr: &str) -> Output {
        Output {
            status: ExitStatus::from_raw(0),
            stdout: stdout.as_bytes().to_vec(),
            stderr: stderr.as_bytes().to_vec(),
        }
    }

    #[test]
    fn parsed_test_failures_are_complete_quality_evidence() {
        assert!(!test_execution_incomplete(false, 3, 1));
        assert!(test_execution_incomplete(false, 0, 0));
        assert!(test_execution_incomplete(false, 3, 0));
        assert!(!test_execution_incomplete(true, 3, 0));
    }

    #[test]
    fn categories_match_runtime_contract() {
        assert_eq!(failure_category(SignalKind::Test), "repo_code_issue");
        assert_eq!(failure_category(SignalKind::Complexity), "repo_setup_issue");
        assert_eq!(failure_category(SignalKind::Deps), "ayni_internal_issue");
    }

    #[test]
    fn prefers_stderr_then_stdout() {
        assert_eq!(combined_output(&output("out", "err")), "err");
        assert_eq!(combined_output(&output("out", "")), "out");
        assert_eq!(
            combined_output(&output("", "")),
            "command failed without stdout/stderr output"
        );
    }

    #[test]
    fn concise_message_is_first_non_empty_line() {
        assert_eq!(
            concise_failure_message(&output("\n\nsecond source", "\nfirst line\nmore")),
            "first line"
        );
    }

    #[test]
    fn execution_error_classification() {
        let spawn = ExecutionError {
            kind: ExecutionErrorKind::Spawn,
            command: String::from("missing-tool"),
            cwd: PathBuf::from("workspace"),
            status: None,
            stdout: Vec::new(),
            stderr: Vec::new(),
            stdout_truncated_bytes: 0,
            stderr_truncated_bytes: 0,
            timeout: None,
            detail: String::from("not found"),
        };
        let timeout = ExecutionError {
            kind: ExecutionErrorKind::Timeout,
            command: String::from("tool check"),
            cwd: PathBuf::from("workspace"),
            status: Some(ExitStatus::from_raw(9 << 8)),
            stdout: b"partial output".to_vec(),
            stderr: b"partial diagnostics".to_vec(),
            stdout_truncated_bytes: 0,
            stderr_truncated_bytes: 0,
            timeout: Some(Duration::from_secs(12)),
            detail: String::new(),
        };

        let spawn_failure = command_failure_from_execution_error(SignalKind::Test, &spawn);
        assert_eq!(spawn_failure.category, "repo_code_issue");
        assert_eq!(spawn_failure.classification, "command_error");
        assert_eq!(spawn_failure.command, "missing-tool");
        assert_eq!(spawn_failure.cwd, "workspace");

        let timeout_failure = command_failure_from_execution_error(SignalKind::Deps, &timeout);
        assert_eq!(timeout_failure.category, "ayni_internal_issue");
        assert_eq!(timeout_failure.classification, "timeout");
        assert_eq!(timeout_failure.exit_code, Some(9));
        assert!(timeout_failure.message.contains("configured timeout: 12s"));
        assert!(timeout_failure.message.contains("partial diagnostics"));
        assert!(timeout_failure.message.contains("partial output"));
    }

    #[test]
    fn output_limit_failure_preserves_truncation_metadata() {
        let error = ExecutionError {
            kind: ExecutionErrorKind::OutputLimit,
            command: String::from("noisy-tool"),
            cwd: PathBuf::from("workspace"),
            status: None,
            stdout: b"retained output".to_vec(),
            stderr: Vec::new(),
            stdout_truncated_bytes: 8192,
            stderr_truncated_bytes: 0,
            timeout: None,
            detail: String::from("stdout exceeded capture limit"),
        };

        let failure = command_failure_from_execution_error(SignalKind::Test, &error);
        assert_eq!(failure.classification, "output_limit");
        assert!(failure.message.contains("retained output"));
        assert!(
            failure
                .message
                .contains("stdout truncated by at least 8192 bytes")
        );

        let mut cancelled = error;
        cancelled.kind = ExecutionErrorKind::Cancelled;
        cancelled.stdout_truncated_bytes = 0;
        let failure = command_failure_from_execution_error(SignalKind::Test, &cancelled);
        assert_eq!(failure.classification, "cancelled");
    }

    #[test]
    fn execution_failure_diagnostics_are_bounded_and_keep_the_tail() {
        let mut stdout = vec![b'h'; super::FAILURE_DIAGNOSTIC_LIMIT * 4];
        stdout.extend_from_slice(b"tail-marker");
        let error = ExecutionError {
            kind: ExecutionErrorKind::Timeout,
            command: String::from("noisy-tool"),
            cwd: PathBuf::from("workspace"),
            status: None,
            stdout,
            stderr: Vec::new(),
            stdout_truncated_bytes: 0,
            stderr_truncated_bytes: 0,
            timeout: Some(Duration::from_secs(12)),
            detail: String::new(),
        };

        let failure = command_failure_from_execution_error(SignalKind::Test, &error);
        assert!(failure.message.contains("captured bytes omitted"));
        assert!(failure.message.ends_with("tail-marker"));
        assert!(failure.message.len() < super::FAILURE_DIAGNOSTIC_LIMIT + 512);
    }

    fn context() -> RunContext {
        let root = PathBuf::from("workspace");
        RunContext {
            repo_root: root.clone(),
            target_root: root.clone(),
            workdir: root.clone(),
            policy: AyniPolicy::default(),
            scope: Scope::default(),
            execution: ExecutionResolution::direct("tool", root, "test", 100),
            cancellation: Default::default(),
            debug: false,
        }
    }

    #[test]
    fn coverage_metric_failure_has_stable_actionable_setup_details() {
        let context = context();
        let missing = coverage_metric_failure(
            &context,
            String::from("coverage-tool"),
            "line_percent",
            ConfiguredMetricEvaluation::Missing,
        )
        .expect("missing configured metric must fail");
        assert_eq!(missing.category, "repo_setup_issue");
        assert_eq!(missing.classification, "missing_coverage_metric");
        assert!(missing.message.contains("`line_percent`"));
        assert!(missing.message.contains("emit it"));

        let unparseable = coverage_metric_failure(
            &context,
            String::from("coverage-tool"),
            "branch_percent",
            ConfiguredMetricEvaluation::Unparseable,
        )
        .expect("unparseable configured metric must fail");
        assert_eq!(unparseable.category, "repo_setup_issue");
        assert_eq!(unparseable.classification, "unparseable_coverage_metric");
        assert!(unparseable.message.contains("`branch_percent`"));
        assert!(unparseable.message.contains("finite percentage"));

        assert!(
            coverage_metric_failure(
                &context,
                String::from("coverage-tool"),
                "line_percent",
                ConfiguredMetricEvaluation::Unconfigured,
            )
            .is_none()
        );
    }
}
