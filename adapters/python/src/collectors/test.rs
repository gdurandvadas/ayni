use super::util::{
    command_failure_from_output, command_for_override_or_default, format_command,
    prepare_report_path, run_command_for_context_streaming_structured,
    run_command_for_context_structured,
};
use ayni_adapters_common::collector::{CollectorError, CollectorResult};
use ayni_adapters_common::failure::test_execution_incomplete;
use ayni_core::{
    Budget, Language, Offenders, RunContext, Scope, SignalKind, SignalResult, SignalRow,
    TestBudget, TestFailure, TestResult, VerificationSelection,
};
use serde::Deserialize;
use std::fs;

#[derive(Debug, Deserialize)]
struct PytestReport {
    duration: Option<f64>,
    summary: Option<PytestSummary>,
    tests: Option<Vec<PytestCase>>,
}

#[derive(Debug, Deserialize)]
struct PytestSummary {
    total: Option<u64>,
    passed: Option<u64>,
    failed: Option<u64>,
    error: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct PytestCase {
    nodeid: Option<String>,
    outcome: Option<String>,
    call: Option<PytestStage>,
    setup: Option<PytestStage>,
    teardown: Option<PytestStage>,
}

#[derive(Debug, Deserialize)]
struct PytestStage {
    crash: Option<PytestCrash>,
    longrepr: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct PytestCrash {
    path: Option<String>,
    lineno: Option<u64>,
    message: Option<String>,
}

pub fn collect(context: &RunContext) -> CollectorResult {
    let report_path =
        prepare_report_path(context, "pytest-report.json").map_err(CollectorError::Adapter)?;
    let report_arg = format!("--json-report-file={}", report_path.display());
    let default_args = ["--json-report", report_arg.as_str()];
    let (program, args) =
        command_for_override_or_default(context, SignalKind::Test, "pytest", &default_args);
    collect_with_command(context, program, args, report_path, None)
}

pub fn collect_selected(
    context: &RunContext,
    selection: &VerificationSelection,
    on_line: &mut dyn FnMut(&str),
) -> CollectorResult {
    let report_path =
        prepare_report_path(context, "pytest-report.json").map_err(CollectorError::Adapter)?;
    let report_arg = format!("--json-report-file={}", report_path.display());
    let default_args = ["--json-report", report_arg.as_str()];
    let (program, mut args) =
        command_for_override_or_default(context, SignalKind::Test, "pytest", &default_args);
    if let Some(file) = &context.scope.file {
        args.push(file.clone());
    } else if let Some(package) = &context.scope.package {
        args.push(package.clone());
    }
    if let Some(name) = &selection.name {
        let selector = format!("::{name}");
        args.push(selector);
    }
    collect_with_command(context, program, args, report_path, Some(on_line))
}

fn collect_with_command(
    context: &RunContext,
    program: String,
    args: Vec<String>,
    report_path: std::path::PathBuf,
    on_line: Option<&mut dyn FnMut(&str)>,
) -> CollectorResult {
    let runner = format_command(&program, &args);
    let output = if let Some(on_line) = on_line {
        run_command_for_context_streaming_structured(context, &program, &args, on_line)?
    } else {
        run_command_for_context_structured(context, &program, &args)?
    };
    let success = output.status.success();

    let report = read_report(&report_path).map_err(|error| {
        if is_no_tests_collected(&output) {
            return String::new();
        }
        if success {
            error
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            format!("{error}; stderr: {}", stderr.trim())
        }
    });
    let report = match report {
        Ok(report) => report,
        Err(error) if error.is_empty() => PytestReport {
            duration: None,
            summary: Some(PytestSummary {
                total: Some(0),
                passed: Some(0),
                failed: Some(0),
                error: Some(0),
            }),
            tests: Some(Vec::new()),
        },
        Err(error) => return Err(CollectorError::Adapter(error)),
    };

    let summary = report.summary.unwrap_or(PytestSummary {
        total: None,
        passed: None,
        failed: None,
        error: None,
    });
    let total_tests = summary.total.unwrap_or(0);
    let passed = summary.passed.unwrap_or(0);
    let failed = summary.failed.unwrap_or(0) + summary.error.unwrap_or(0);
    let duration_ms = report.duration.map(|value| (value * 1000.0) as u64);
    let failure = test_execution_incomplete(success, total_tests, failed)
        .then(|| command_failure_from_output(context, SignalKind::Test, &program, &args, &output));
    let mut offenders = report
        .tests
        .unwrap_or_default()
        .into_iter()
        .filter(|case| matches!(case.outcome.as_deref(), Some("failed" | "error")))
        .map(test_failure)
        .collect::<Vec<_>>();
    if success && total_tests == 0 {
        offenders.push(zero_tests_failure());
    }

    Ok(SignalRow {
        kind: SignalKind::Test,
        language: Language::Python,
        scope: Scope {
            workspace_root: context.scope.workspace_root.clone(),
            path: context.scope.path.clone(),
            package: context.scope.package.clone(),
            file: context.scope.file.clone(),
        },
        pass: test_row_passes(success, total_tests, failed),
        result: SignalResult::Test(TestResult {
            total_tests,
            passed,
            failed,
            duration_ms,
            runner,
            failure,
        }),
        budget: Budget::Test(TestBudget::default()),
        offenders: Offenders::Test(offenders),
    })
}

fn zero_tests_failure() -> TestFailure {
    TestFailure {
        file: None,
        line: None,
        message: String::from(
            "test runner completed successfully but discovered zero tests; add tests or correct the test selection",
        ),
        test_name: None,
    }
}

fn test_row_passes(success: bool, total_tests: u64, failed: u64) -> bool {
    success && total_tests > 0 && failed == 0
}

fn is_no_tests_collected(output: &std::process::Output) -> bool {
    if output.status.code() == Some(5) {
        return true;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    stdout.contains("no tests ran") || stderr.contains("no tests ran")
}

fn read_report(path: &std::path::Path) -> Result<PytestReport, String> {
    let content = fs::read_to_string(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    serde_json::from_str(&content)
        .map_err(|error| format!("failed to parse {}: {error}", path.display()))
}

fn test_failure(case: PytestCase) -> TestFailure {
    let stage = case.call.or(case.setup).or(case.teardown);
    let crash = stage.as_ref().and_then(|stage| stage.crash.as_ref());
    let message = crash
        .and_then(|crash| crash.message.clone())
        .or_else(|| {
            stage
                .as_ref()
                .and_then(|stage| stage.longrepr.as_ref())
                .map(|value| value.to_string())
        })
        .unwrap_or_else(|| String::from("pytest case failed"));
    TestFailure {
        file: crash.and_then(|crash| crash.path.clone()).or_else(|| {
            case.nodeid
                .as_ref()
                .and_then(|nodeid| nodeid.split("::").next())
                .map(String::from)
        }),
        line: crash.and_then(|crash| crash.lineno),
        message,
        test_name: case.nodeid,
    }
}

#[cfg(test)]
mod tests {
    use super::{test_row_passes, zero_tests_failure};

    #[test]
    fn successful_zero_test_run_fails_with_an_actionable_finding() {
        assert!(!test_row_passes(true, 0, 0));
        assert!(
            zero_tests_failure()
                .message
                .contains("discovered zero tests")
        );
    }
}

#[cfg(test)]
mod report_tests {
    use super::{
        PytestCase, PytestCrash, PytestStage, collect_with_command, is_no_tests_collected,
        read_report, test_failure,
    };
    use ayni_core::{AyniPolicy, ExecutionResolution, RunContext, Scope, SignalResult};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use tempfile::TempDir;

    fn context(root: &Path) -> RunContext {
        RunContext {
            repo_root: root.to_path_buf(),
            target_root: root.to_path_buf(),
            workdir: root.to_path_buf(),
            policy: AyniPolicy::default(),
            scope: Scope::default(),
            execution: ExecutionResolution::direct("sh", root.to_path_buf(), "test", 100),
            cancellation: Default::default(),
            debug: false,
        }
    }

    fn script(root: &Path, report: &Path, body: &str) -> PathBuf {
        let path = root.join("pytest-fixture.sh");
        fs::write(
            &path,
            format!(
                "#!/bin/sh\ncat > {} <<'REPORT'\n{}\nREPORT\n{}\n",
                report.display(),
                body,
                "exit 0"
            ),
        )
        .expect("fixture script");
        path
    }

    #[test]
    fn fixture_report_accounts_for_passed_failed_and_error_cases() {
        let temp = TempDir::new().expect("temporary repository");
        let report = temp.path().join("pytest-report.json");
        let fixture = r#"{"duration":1.25,"summary":{"total":3,"passed":1,"failed":1,"error":1},"tests":[{"nodeid":"tests/test_a.py::test_passes","outcome":"passed"},{"nodeid":"tests/test_a.py::test_fails","outcome":"failed","call":{"crash":{"path":"tests/test_a.py","lineno":12,"message":"assert 1 == 2"}}},{"nodeid":"tests/test_b.py::test_setup","outcome":"error","setup":{"longrepr":"setup exploded"}}]}"#;
        let command = script(temp.path(), &report, fixture);

        let row = collect_with_command(
            &context(temp.path()),
            String::from("sh"),
            vec![command.display().to_string()],
            report,
            None,
        )
        .expect("parsed report");
        let SignalResult::Test(result) = row.result else {
            panic!("test result")
        };
        assert_eq!(
            (
                result.total_tests,
                result.passed,
                result.failed,
                result.duration_ms
            ),
            (3, 1, 2, Some(1250))
        );
        assert!(!row.pass);
        let ayni_core::Offenders::Test(offenders) = row.offenders else {
            panic!("test offenders")
        };
        assert_eq!(offenders.len(), 2);
        assert_eq!(offenders[0].file.as_deref(), Some("tests/test_a.py"));
        assert_eq!(offenders[1].message, "\"setup exploded\"");
    }

    #[test]
    fn no_tests_exit_without_a_report_is_handled_as_an_empty_run() {
        let output = Command::new("sh")
            .args(["-c", "printf 'no tests ran\\n'; exit 5"])
            .output()
            .expect("shell output");
        assert!(is_no_tests_collected(&output));

        let temp = TempDir::new().expect("temporary repository");
        let report = temp.path().join("missing.json");
        let row = collect_with_command(
            &context(temp.path()),
            String::from("sh"),
            vec![
                String::from("-c"),
                String::from("printf 'no tests ran\\n'; exit 5"),
            ],
            report,
            None,
        )
        .expect("no-tests row");
        let SignalResult::Test(result) = row.result else {
            panic!("test result")
        };
        assert_eq!(
            (result.total_tests, result.passed, result.failed),
            (0, 0, 0)
        );
        assert!(!row.pass);
    }

    #[test]
    fn malformed_and_missing_reports_are_reported_with_paths() {
        let temp = TempDir::new().expect("temporary repository");
        let missing = temp.path().join("missing.json");
        assert!(
            read_report(&missing)
                .expect_err("missing report")
                .contains("failed to read")
        );
        let malformed = temp.path().join("malformed.json");
        fs::write(&malformed, "{").expect("malformed fixture");
        assert!(
            read_report(&malformed)
                .expect_err("malformed report")
                .contains("failed to parse")
        );
    }

    #[test]
    fn failed_cases_prefer_call_crash_and_fall_back_to_nodeid() {
        let failure = test_failure(PytestCase {
            nodeid: Some(String::from("tests/test_api.py::test_create")),
            outcome: Some(String::from("failed")),
            call: Some(PytestStage {
                crash: Some(PytestCrash {
                    path: Some(String::from("src/api.py")),
                    lineno: Some(42),
                    message: Some(String::from("expected success")),
                }),
                longrepr: None,
            }),
            setup: None,
            teardown: None,
        });
        assert_eq!(failure.file.as_deref(), Some("src/api.py"));
        assert_eq!(failure.line, Some(42));
        assert_eq!(failure.message, "expected success");

        let fallback = test_failure(PytestCase {
            nodeid: Some(String::from("tests/test_api.py::test_fallback")),
            outcome: Some(String::from("error")),
            call: None,
            setup: None,
            teardown: None,
        });
        assert_eq!(fallback.file.as_deref(), Some("tests/test_api.py"));
        assert_eq!(fallback.message, "pytest case failed");
    }
}
