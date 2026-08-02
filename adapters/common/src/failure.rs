//! Shared command-failure classification and `CommandFailure` construction.

use crate::exec::format_command;
use ayni_core::{CommandFailure, ConfiguredMetricEvaluation, RunContext, SignalKind};
use std::process::Output;

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
        combined_output, concise_failure_message, coverage_metric_failure, failure_category,
    };
    use ayni_core::{
        AyniPolicy, ConfiguredMetricEvaluation, ExecutionResolution, RunContext, Scope, SignalKind,
    };
    use std::os::unix::process::ExitStatusExt;
    use std::path::PathBuf;
    use std::process::{ExitStatus, Output};

    fn output(stdout: &str, stderr: &str) -> Output {
        Output {
            status: ExitStatus::from_raw(0),
            stdout: stdout.as_bytes().to_vec(),
            stderr: stderr.as_bytes().to_vec(),
        }
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

    fn context() -> RunContext {
        let root = PathBuf::from("workspace");
        RunContext {
            repo_root: root.clone(),
            target_root: root.clone(),
            workdir: root.clone(),
            policy: AyniPolicy::default(),
            scope: Scope::default(),
            execution: ExecutionResolution::direct("tool", root, "test", 100),
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
