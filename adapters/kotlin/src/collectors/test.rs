use super::util::{find_reports, gradle_command, prepare_gradle_execution, report_root};
use ayni_adapters_common::collector::{CollectorError, CollectorResult};
use ayni_adapters_common::exec::{format_command, run_command_for_context_structured};
use ayni_adapters_common::failure::{command_failure_from_output, test_execution_incomplete};
use ayni_adapters_common::xml::{attr_f64, attr_string, attr_u64};
use ayni_core::{
    Budget, Language, Offenders, RunContext, SignalKind, SignalResult, SignalRow, TestBudget,
    TestFailure, TestResult, VerificationSelection,
};
use regex::Regex;
use std::fs;
use std::path::PathBuf;

pub fn collect(context: &RunContext) -> CollectorResult {
    prepare_gradle_execution(context, SignalKind::Test).map_err(CollectorError::Adapter)?;
    let (program, args) = gradle_command(context, SignalKind::Test, "test");
    collect_with_command(context, program, args)
}

pub fn collect_selected(
    context: &RunContext,
    selection: &VerificationSelection,
    _on_line: &mut dyn FnMut(&str),
) -> CollectorResult {
    if context.scope.file.is_some() {
        return Err(CollectorError::Adapter(String::from(
            "Kotlin source-file selection is unsupported; use --package and optional --name",
        )));
    }
    prepare_gradle_execution(context, SignalKind::Test).map_err(CollectorError::Adapter)?;
    let (program, mut args) = gradle_command(context, SignalKind::Test, "test");
    let selector = match (&context.scope.package, &selection.name) {
        (Some(package), Some(name)) => format!("{package}.{name}"),
        (Some(package), None) => package.clone(),
        (None, Some(name)) => name.clone(),
        (None, None) => {
            return Err(CollectorError::Adapter(String::from(
                "a package or test name is required",
            )));
        }
    };
    args.extend([String::from("--tests"), selector]);
    collect_with_command(context, program, args)
}

fn collect_with_command(
    context: &RunContext,
    program: String,
    args: Vec<String>,
) -> CollectorResult {
    let runner = format_command(&program, &args);
    let output = run_command_for_context_structured(context, &program, &args)?;
    let report_paths = find_reports(
        &report_root(context, SignalKind::Test),
        &["build", "test-results", "test"],
        "xml",
    );
    Ok(build_row_from_output(
        context,
        &program,
        &args,
        output,
        &report_paths,
        runner,
    ))
}

pub(super) fn build_row_from_output(
    context: &RunContext,
    program: &str,
    args: &[String],
    output: std::process::Output,
    report_paths: &[PathBuf],
    runner: String,
) -> SignalRow {
    let report = match parse_reports(report_paths) {
        Ok(report) => report,
        Err(message) => return evidence_error_row(context, runner, message),
    };
    let failed = report.failures + report.errors;
    let execution_incomplete =
        test_execution_incomplete(output.status.success(), report.tests, failed);
    let mut offenders = report.offenders;
    if output.status.success() && report.tests == 0 {
        offenders.push(zero_tests_failure());
    }

    SignalRow {
        kind: SignalKind::Test,
        language: Language::Kotlin,
        scope: context.scope.clone(),
        pass: test_row_passes(output.status.success(), report.tests, failed),
        result: SignalResult::Test(TestResult {
            total_tests: report.tests,
            passed: report
                .tests
                .saturating_sub(failed)
                .saturating_sub(report.skipped),
            failed,
            duration_ms: report.duration_ms,
            runner: runner.clone(),
            failure: execution_incomplete.then(|| {
                command_failure_from_output(context, SignalKind::Test, program, args, &output)
            }),
        }),
        budget: Budget::Test(TestBudget::default()),
        offenders: Offenders::Test(offenders),
    }
}

fn evidence_error_row(context: &RunContext, runner: String, message: String) -> SignalRow {
    SignalRow {
        kind: SignalKind::Test,
        language: Language::Kotlin,
        scope: context.scope.clone(),
        pass: false,
        result: SignalResult::Test(TestResult {
            total_tests: 0,
            passed: 0,
            failed: 0,
            duration_ms: None,
            runner: runner.clone(),
            failure: Some(ayni_adapters_common::failure::setup_failure(
                context, runner, message,
            )),
        }),
        budget: Budget::Test(TestBudget::default()),
        offenders: Offenders::Test(Vec::new()),
    }
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

#[derive(Default)]
struct JunitSummary {
    tests: u64,
    failures: u64,
    errors: u64,
    skipped: u64,
    duration_ms: Option<u64>,
    offenders: Vec<TestFailure>,
}

fn parse_reports(paths: &[PathBuf]) -> Result<JunitSummary, String> {
    let mut summary = JunitSummary::default();
    for path in paths {
        let content = fs::read_to_string(path)
            .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
        let parsed = parse_junit_xml(&content)?;
        summary.tests = checked_count_add(summary.tests, parsed.tests, "tests")?;
        summary.failures = checked_count_add(summary.failures, parsed.failures, "failures")?;
        summary.errors = checked_count_add(summary.errors, parsed.errors, "errors")?;
        summary.skipped = checked_count_add(summary.skipped, parsed.skipped, "skipped")?;
        summary.duration_ms = Some(checked_count_add(
            summary.duration_ms.unwrap_or(0),
            parsed.duration_ms.unwrap_or(0),
            "duration",
        )?);
        summary.offenders.extend(parsed.offenders);
    }
    if summary.duration_ms == Some(0) {
        summary.duration_ms = None;
    }
    Ok(summary)
}

fn validate_complete_elements(
    content: &str,
    element: &str,
    required: bool,
) -> Result<usize, String> {
    let opening_re = Regex::new(&format!(r#"<{element}\b[^>]*>"#))
        .map_err(|error| format!("failed to compile {element} opening regex: {error}"))?;
    let closing_re = Regex::new(&format!(r#"</{element}\s*>"#))
        .map_err(|error| format!("failed to compile {element} closing regex: {error}"))?;
    let opening_tags = opening_re
        .find_iter(content)
        .map(|matched| matched.as_str())
        .collect::<Vec<_>>();
    let self_closing = opening_tags
        .iter()
        .filter(|tag| tag.trim_end().ends_with("/>"))
        .count();
    let closed = closing_re.find_iter(content).count();
    if (required && opening_tags.is_empty()) || opening_tags.len() != self_closing + closed {
        return Err(format!(
            "JUnit evidence did not contain complete {element} XML"
        ));
    }
    Ok(opening_tags.len())
}

fn checked_count_add(current: u64, value: u64, field: &str) -> Result<u64, String> {
    current
        .checked_add(value)
        .ok_or_else(|| format!("JUnit {field} count overflowed"))
}

fn parse_junit_xml(content: &str) -> Result<JunitSummary, String> {
    let testsuite_re = Regex::new(r#"<testsuite\b([^>]*)>"#)
        .map_err(|error| format!("failed to compile testsuite regex: {error}"))?;
    let testcase_re = Regex::new(r#"(?s)<testcase\b([^>]*)>(.*?)</testcase>"#)
        .map_err(|error| format!("failed to compile testcase regex: {error}"))?;
    let failure_re = Regex::new(r#"(?s)<(failure|error)\b([^>]*)>(.*?)</(failure|error)>"#)
        .map_err(|error| format!("failed to compile failure regex: {error}"))?;
    let skipped_re = Regex::new(r#"<skipped\b"#)
        .map_err(|error| format!("failed to compile skipped regex: {error}"))?;
    validate_complete_elements(content, "testsuite", true)?;
    let testcase_count = validate_complete_elements(content, "testcase", false)?;
    let mut summary = JunitSummary::default();
    for caps in testsuite_re.captures_iter(content) {
        let attrs = caps.get(1).map(|value| value.as_str()).unwrap_or("");
        summary.tests = checked_count_add(
            summary.tests,
            attr_u64(attrs, "tests").unwrap_or(0),
            "tests",
        )?;
        summary.failures = checked_count_add(
            summary.failures,
            attr_u64(attrs, "failures").unwrap_or(0),
            "failures",
        )?;
        summary.errors = checked_count_add(
            summary.errors,
            attr_u64(attrs, "errors").unwrap_or(0),
            "errors",
        )?;
        summary.skipped = checked_count_add(
            summary.skipped,
            attr_u64(attrs, "skipped").unwrap_or(0),
            "skipped",
        )?;
        if let Some(seconds) = attr_f64(attrs, "time") {
            if !seconds.is_finite() || seconds < 0.0 {
                return Err(String::from(
                    "JUnit duration was not a finite non-negative value",
                ));
            }
            summary.duration_ms = Some(checked_count_add(
                summary.duration_ms.unwrap_or(0),
                (seconds * 1000.0) as u64,
                "duration",
            )?);
        }
    }
    let mut testcase_skipped = 0;
    for caps in testcase_re.captures_iter(content) {
        let attrs = caps.get(1).map(|value| value.as_str()).unwrap_or("");
        let body = caps.get(2).map(|value| value.as_str()).unwrap_or("");
        for failure in failure_re.captures_iter(body) {
            let failure_attrs = failure.get(2).map(|value| value.as_str()).unwrap_or("");
            let message = attr_string(failure_attrs, "message").unwrap_or_else(|| {
                failure
                    .get(3)
                    .map(|value| value.as_str().trim().to_string())
                    .filter(|value| !value.is_empty())
                    .unwrap_or_else(|| String::from("JUnit test failed"))
            });
            summary.offenders.push(TestFailure {
                file: attr_string(attrs, "classname"),
                line: None,
                message,
                test_name: attr_string(attrs, "name"),
            });
        }
        if skipped_re.is_match(body) {
            testcase_skipped += 1;
        }
    }
    summary.skipped = summary.skipped.max(testcase_skipped);
    if summary.tests == 0 {
        summary.tests = testcase_count as u64;
    }
    if summary.tests != testcase_count as u64 {
        return Err(format!(
            "JUnit declared {} tests but contained {testcase_count} testcase elements",
            summary.tests
        ));
    }
    let failed = summary
        .failures
        .checked_add(summary.errors)
        .ok_or_else(|| String::from("JUnit failed test counts overflowed"))?;
    let accounted = failed
        .checked_add(summary.skipped)
        .ok_or_else(|| String::from("JUnit accounted test counts overflowed"))?;
    if accounted > summary.tests {
        return Err(format!(
            "JUnit failures, errors, and skipped counts ({accounted}) exceed tests ({})",
            summary.tests
        ));
    }
    if failed == 0 && !summary.offenders.is_empty() {
        summary.failures = summary.offenders.len() as u64;
        if summary
            .failures
            .checked_add(summary.skipped)
            .is_none_or(|accounted| accounted > summary.tests)
        {
            return Err(String::from(
                "JUnit failure elements exceed the declared test count",
            ));
        }
    }
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::{parse_junit_xml, test_row_passes, zero_tests_failure};

    #[test]
    fn parses_junit_failures() {
        let summary = parse_junit_xml(
            r#"<testsuite tests="2" failures="1" errors="0" skipped="0" time="1.5">
<testcase classname="AppTest" name="ok"></testcase>
<testcase classname="AppTest" name="fails"><failure message="broken">trace</failure></testcase>
</testsuite>"#,
        )
        .expect("junit");

        assert_eq!(summary.tests, 2);
        assert_eq!(summary.failures, 1);
        assert_eq!(summary.duration_ms, Some(1500));
        assert_eq!(summary.offenders[0].test_name.as_deref(), Some("fails"));
    }

    #[test]
    fn suite_and_testcase_skipped_counts_are_not_double_counted() {
        let summary = parse_junit_xml(
            r#"<testsuite tests="2" failures="0" errors="0" skipped="1">
<testcase classname="AppTest" name="runs"></testcase>
<testcase classname="AppTest" name="skips"><skipped/></testcase>
</testsuite>"#,
        )
        .expect("junit");

        assert_eq!(summary.tests, 2);
        assert_eq!(summary.skipped, 1);
    }

    #[test]
    fn rejects_truncated_and_inconsistent_junit_evidence() {
        assert!(parse_junit_xml(r#"<testsuite tests="1" failures="0" errors="0">"#).is_err());
        assert!(
            parse_junit_xml(
                r#"<testsuite tests="1" failures="1" errors="1" skipped="0"><testcase name="a"/></testsuite>"#,
            )
            .is_err()
        );
        assert!(
            parse_junit_xml(
                r#"<testsuite tests="2" failures="0" errors="0" skipped="0"><testcase name="a"/></testsuite>"#,
            )
            .is_err()
        );
    }

    #[test]
    fn zero_test_finding_is_actionable() {
        assert!(!test_row_passes(true, 0, 0));
        assert!(
            zero_tests_failure()
                .message
                .contains("discovered zero tests")
        );
    }
}
