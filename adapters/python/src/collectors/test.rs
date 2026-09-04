use super::util::{
    command_failure_from_output, command_for_override_or_default, format_command,
    prepare_report_path, run_command_for_context_streaming_structured,
    run_command_for_context_structured,
};
use ayni_adapters_common::collector::{CollectorError, CollectorResult};
use ayni_adapters_common::failure::{setup_failure, test_execution_incomplete};
use ayni_core::{
    Budget, Language, Offenders, RunContext, SignalKind, SignalResult, SignalRow, TestBudget,
    TestFailure, TestResult, VerificationSelection,
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
    xfailed: Option<u64>,
    xpassed: Option<u64>,
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
    outcome: Option<String>,
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
    append_test_selection(context, selection, &mut args);
    collect_with_command(context, program, args, report_path, Some(on_line))
}

fn append_test_selection(
    context: &RunContext,
    selection: &VerificationSelection,
    args: &mut Vec<String>,
) {
    if let Some(file) = &context.scope.file {
        args.push(
            selection
                .name
                .as_ref()
                .map_or_else(|| file.clone(), |name| format!("{file}::{name}")),
        );
        return;
    }
    if let Some(package) = &context.scope.package {
        args.push(package.clone());
    }
    if let Some(name) = &selection.name {
        args.extend([String::from("-k"), name.clone()]);
    }
}

fn collect_with_command(
    context: &RunContext,
    program: String,
    args: Vec<String>,
    report_path: std::path::PathBuf,
    on_line: Option<&mut dyn FnMut(&str)>,
) -> CollectorResult {
    let output = if let Some(on_line) = on_line {
        run_command_for_context_streaming_structured(context, &program, &args, on_line)?
    } else {
        run_command_for_context_structured(context, &program, &args)?
    };
    Ok(build_row_from_output(
        context,
        &program,
        &args,
        &report_path,
        &output,
    ))
}

pub(super) fn build_row_from_output(
    context: &RunContext,
    program: &str,
    args: &[String],
    report_path: &std::path::Path,
    output: &std::process::Output,
) -> SignalRow {
    let runner = format_command(program, args);
    let success = output.status.success();
    let report = read_report(report_path);
    let (report, mut report_failure) = match report {
        Ok(report) => (report, None),
        Err(_) if is_no_tests_collected(output) => (
            PytestReport {
                duration: None,
                summary: Some(PytestSummary {
                    total: Some(0),
                    passed: Some(0),
                    failed: Some(0),
                    error: Some(0),
                    xfailed: Some(0),
                    xpassed: Some(0),
                }),
                tests: Some(Vec::new()),
            },
            None,
        ),
        Err(error) => (
            PytestReport {
                duration: None,
                summary: None,
                tests: None,
            },
            Some(setup_failure(
                context,
                runner.clone(),
                format!("pytest command did not produce a parseable JSON report: {error}"),
            )),
        ),
    };

    let summary = report.summary.unwrap_or(PytestSummary {
        total: None,
        passed: None,
        failed: None,
        error: None,
        xfailed: None,
        xpassed: None,
    });
    let total_tests = summary.total.unwrap_or(0);
    let passed = summary
        .passed
        .unwrap_or(0)
        .checked_add(summary.xpassed.unwrap_or(0));
    let reported_passed = passed.unwrap_or(u64::MAX);
    let failed_cases = summary.failed.unwrap_or(0);
    let error_cases = summary.error.unwrap_or(0);
    let failed = failed_cases.saturating_add(error_cases);
    let cases = report.tests;
    let summary_valid = passed.is_some()
        && test_counts_valid(total_tests, reported_passed, failed_cases, error_cases);
    let case_counts = cases.as_deref().and_then(parsed_case_counts);
    if (!summary_valid || !summary_matches_cases(&summary, case_counts)) && report_failure.is_none()
    {
        let message = if !summary_valid {
            format!(
                "pytest JSON summary counts were inconsistent: total={total_tests}, passed={reported_passed}, failed={failed_cases}, error={error_cases}"
            )
        } else {
            String::from("pytest JSON summary counts did not match parsed test case outcomes")
        };
        report_failure = Some(setup_failure(context, runner.clone(), message));
    }
    let duration_ms = report.duration.map(|value| (value * 1000.0) as u64);
    let report_complete = report_failure.is_none();
    let failure = report_failure.or_else(|| {
        test_execution_incomplete(success, total_tests, failed)
            .then(|| command_failure_from_output(context, SignalKind::Test, program, args, output))
    });
    let mut offenders = cases
        .unwrap_or_default()
        .into_iter()
        .filter(|case| matches!(case.outcome.as_deref(), Some("failed" | "error")))
        .map(test_failure)
        .collect::<Vec<_>>();
    if success && total_tests == 0 && failure.is_none() {
        offenders.push(zero_tests_failure());
    }

    SignalRow {
        kind: SignalKind::Test,
        language: Language::Python,
        scope: context.scope.clone(),
        pass: report_complete && test_row_passes(success, total_tests, failed),
        result: SignalResult::Test(TestResult {
            total_tests,
            passed: reported_passed,
            failed,
            duration_ms,
            runner,
            failure,
        }),
        budget: Budget::Test(TestBudget::default()),
        offenders: Offenders::Test(offenders),
    }
}

#[derive(Debug, PartialEq, Eq)]
struct PytestCaseCounts {
    total: u64,
    passed: u64,
    failed: u64,
    errors: u64,
    xfailed: u64,
    xpassed: u64,
}

fn parsed_case_counts(cases: &[PytestCase]) -> Option<PytestCaseCounts> {
    let mut counts = PytestCaseCounts {
        total: 0,
        passed: 0,
        failed: 0,
        errors: 0,
        xfailed: 0,
        xpassed: 0,
    };
    for case in cases {
        increment_count(&mut counts.total)?;
        record_case_outcome(&mut counts, case)?;
    }
    Some(counts)
}

fn record_case_outcome(counts: &mut PytestCaseCounts, case: &PytestCase) -> Option<()> {
    match case.outcome.as_deref()? {
        "passed" => increment_count(&mut counts.passed),
        "xpassed" => increment_count(&mut counts.xpassed),
        "failed" if has_setup_or_teardown_error(case) => increment_count(&mut counts.errors),
        "failed" => increment_count(&mut counts.failed),
        "error" => increment_count(&mut counts.errors),
        "xfailed" => increment_count(&mut counts.xfailed),
        "skipped" => Some(()),
        _ => None,
    }
}

fn increment_count(count: &mut u64) -> Option<()> {
    *count = count.checked_add(1)?;
    Some(())
}

fn has_setup_or_teardown_error(case: &PytestCase) -> bool {
    [case.setup.as_ref(), case.teardown.as_ref()]
        .into_iter()
        .flatten()
        .any(|stage| matches!(stage.outcome.as_deref(), Some("failed" | "error")))
}

fn summary_matches_cases(summary: &PytestSummary, cases: Option<PytestCaseCounts>) -> bool {
    let Some(cases) = cases else {
        return false;
    };
    summary.total == Some(cases.total)
        && summary.passed.unwrap_or(0) == cases.passed
        && summary.failed.unwrap_or(0) == cases.failed
        && summary.error.unwrap_or(0) == cases.errors
        && summary.xfailed.unwrap_or(0) == cases.xfailed
        && summary.xpassed.unwrap_or(0) == cases.xpassed
}

fn test_counts_valid(total: u64, passed: u64, failed: u64, errors: u64) -> bool {
    passed
        .checked_add(failed)
        .and_then(|accounted| accounted.checked_add(errors))
        .is_some_and(|accounted| accounted <= total)
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
    let stages = [
        case.setup.as_ref(),
        case.call.as_ref(),
        case.teardown.as_ref(),
    ];
    let stage = stages
        .into_iter()
        .flatten()
        .find(|stage| matches!(stage.outcome.as_deref(), Some("failed" | "error")))
        .or(case.call.as_ref())
        .or(case.setup.as_ref())
        .or(case.teardown.as_ref());
    let crash = stage.and_then(|stage| stage.crash.as_ref());
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
    use super::{
        PytestCase, PytestStage, PytestSummary, parsed_case_counts, summary_matches_cases,
        test_counts_valid, test_row_passes, zero_tests_failure,
    };

    #[test]
    fn rejects_inconsistent_or_overflowing_summary_counts() {
        assert!(test_counts_valid(3, 1, 1, 1));
        assert!(!test_counts_valid(1, 2, 0, 0));
        assert!(!test_counts_valid(1, u64::MAX, 1, 0));
        assert!(!test_counts_valid(1, 0, u64::MAX, 1));
    }

    #[test]
    fn requires_summary_counts_to_match_parsed_case_outcomes() {
        let cases = [
            PytestCase {
                nodeid: None,
                outcome: Some(String::from("passed")),
                call: None,
                setup: None,
                teardown: None,
            },
            PytestCase {
                nodeid: None,
                outcome: Some(String::from("failed")),
                call: None,
                setup: None,
                teardown: None,
            },
            PytestCase {
                nodeid: None,
                outcome: Some(String::from("skipped")),
                call: None,
                setup: None,
                teardown: None,
            },
        ];
        let matching = PytestSummary {
            total: Some(3),
            passed: Some(1),
            failed: Some(1),
            error: Some(0),
            xfailed: None,
            xpassed: None,
        };
        assert!(summary_matches_cases(&matching, parsed_case_counts(&cases)));
        let omitted_zero_categories = PytestSummary {
            total: Some(1),
            passed: Some(1),
            failed: None,
            error: None,
            xfailed: None,
            xpassed: None,
        };
        assert!(summary_matches_cases(
            &omitted_zero_categories,
            parsed_case_counts(&cases[..1])
        ));
        let mismatched = PytestSummary {
            failed: Some(0),
            ..matching
        };
        assert!(!summary_matches_cases(
            &mismatched,
            parsed_case_counts(&cases)
        ));
        assert!(
            parsed_case_counts(&[PytestCase {
                nodeid: None,
                outcome: None,
                call: None,
                setup: None,
                teardown: None
            }])
            .is_none()
        );
    }

    #[test]
    fn accepts_native_xfail_and_xpass_outcomes() {
        let cases = [
            PytestCase {
                nodeid: None,
                outcome: Some(String::from("xfailed")),
                call: None,
                setup: None,
                teardown: None,
            },
            PytestCase {
                nodeid: None,
                outcome: Some(String::from("xpassed")),
                call: None,
                setup: None,
                teardown: None,
            },
        ];
        let summary = PytestSummary {
            total: Some(2),
            passed: None,
            failed: None,
            error: None,
            xfailed: Some(1),
            xpassed: Some(1),
        };
        assert!(summary_matches_cases(&summary, parsed_case_counts(&cases)));
    }

    #[test]
    fn treats_failed_cases_with_setup_or_teardown_failures_as_errors() {
        let cases = [PytestCase {
            nodeid: None,
            outcome: Some(String::from("failed")),
            call: None,
            setup: Some(PytestStage {
                outcome: Some(String::from("failed")),
                crash: None,
                longrepr: None,
            }),
            teardown: None,
        }];
        let summary = PytestSummary {
            total: Some(1),
            passed: None,
            failed: None,
            error: Some(1),
            xfailed: None,
            xpassed: None,
        };
        assert!(summary_matches_cases(&summary, parsed_case_counts(&cases)));
    }

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
        PytestCase, PytestCrash, PytestStage, append_test_selection, collect_with_command,
        is_no_tests_collected, read_report, test_failure,
    };
    use ayni_core::{
        AyniPolicy, ExecutionResolution, RunContext, Scope, SignalResult, VerificationSelection,
    };
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
    fn selected_file_and_test_name_form_one_pytest_node_id() {
        let root = TempDir::new().expect("fixture");
        let mut context = context(root.path());
        context.scope.file = Some(String::from("tests/test_api.py"));
        let selection = VerificationSelection {
            name: Some(String::from("test_create")),
            ..VerificationSelection::default()
        };
        let mut args = Vec::new();
        append_test_selection(&context, &selection, &mut args);
        assert_eq!(args, ["tests/test_api.py::test_create"]);
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
    fn inconsistent_summary_counts_produce_failed_typed_evidence() {
        let temp = TempDir::new().expect("temporary repository");
        let report = temp.path().join("pytest-report.json");
        let command = script(
            temp.path(),
            &report,
            r#"{"summary":{"total":1,"passed":2,"failed":0,"error":0},"tests":[]}"#,
        );

        let row = collect_with_command(
            &context(temp.path()),
            String::from("sh"),
            vec![command.display().to_string()],
            report,
            None,
        )
        .expect("typed failed row");
        assert!(!row.pass);
        assert!(
            row.result
                .command_failure()
                .expect("count failure")
                .message
                .contains("counts were inconsistent")
        );
    }

    #[test]
    fn mismatched_case_outcomes_produce_a_typed_failed_row_without_discarding_counts_or_offenders()
    {
        let temp = TempDir::new().expect("temporary repository");
        let report = temp.path().join("pytest-report.json");
        let command = script(
            temp.path(),
            &report,
            r#"{"summary":{"total":1,"passed":1,"failed":0,"error":0},"tests":[{"nodeid":"tests/test_a.py::test_fails","outcome":"failed"}]}"#,
        );
        let row = collect_with_command(
            &context(temp.path()),
            String::from("sh"),
            vec![command.display().to_string()],
            report,
            None,
        )
        .expect("typed failed row");
        let SignalResult::Test(result) = row.result else {
            panic!("test result")
        };
        assert_eq!(
            (result.total_tests, result.passed, result.failed),
            (1, 1, 0)
        );
        assert!(
            result
                .failure
                .expect("evidence failure")
                .message
                .contains("did not match")
        );
        let ayni_core::Offenders::Test(offenders) = row.offenders else {
            panic!("test offenders")
        };
        assert_eq!(offenders.len(), 1);
        assert_eq!(
            offenders[0].test_name.as_deref(),
            Some("tests/test_a.py::test_fails")
        );
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
                outcome: Some(String::from("failed")),
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

        let teardown = test_failure(PytestCase {
            nodeid: Some(String::from("tests/test_api.py::test_cleanup")),
            outcome: Some(String::from("error")),
            call: Some(PytestStage {
                outcome: Some(String::from("passed")),
                crash: None,
                longrepr: None,
            }),
            setup: None,
            teardown: Some(PytestStage {
                outcome: Some(String::from("failed")),
                crash: Some(PytestCrash {
                    path: Some(String::from("tests/conftest.py")),
                    lineno: Some(9),
                    message: Some(String::from("cleanup failed")),
                }),
                longrepr: None,
            }),
        });
        assert_eq!(teardown.file.as_deref(), Some("tests/conftest.py"));
        assert_eq!(teardown.message, "cleanup failed");

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
