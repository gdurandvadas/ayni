use super::util::{find_reports, gradle_command, prepare_gradle_execution, report_root};
use super::xml::XmlDocument;
use ayni_adapters_common::collector::{CollectorError, CollectorResult};
use ayni_adapters_common::exec::{format_command, run_command_for_context_structured};
use ayni_adapters_common::failure::{command_failure_from_output, test_execution_incomplete};
use ayni_adapters_common::xml::attr_string;
use ayni_core::{
    Budget, Language, Offenders, RunContext, SignalKind, SignalResult, SignalRow, TestBudget,
    TestFailure, TestResult, VerificationSelection,
};
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
        Err(message) if message == "missing JUnit XML evidence" => {
            return missing_junit_evidence_row(context, runner);
        }
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

fn missing_junit_evidence_row(context: &RunContext, runner: String) -> SignalRow {
    let mut row = evidence_error_row(
        context,
        runner,
        String::from("test command completed but no JUnit XML report was generated"),
    );
    if let SignalResult::Test(result) = &mut row.result
        && let Some(failure) = &mut result.failure
    {
        failure.classification = String::from("missing_junit_report");
    }
    row
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
    if paths.is_empty() {
        return Err(String::from("missing JUnit XML evidence"));
    }
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

fn checked_count_add(current: u64, value: u64, field: &str) -> Result<u64, String> {
    current
        .checked_add(value)
        .ok_or_else(|| format!("JUnit {field} count overflowed"))
}

fn parse_junit_xml(content: &str) -> Result<JunitSummary, String> {
    let document = XmlDocument::parse(content)?;
    let suite_indices = elements_named(&document, "testsuite");
    if suite_indices.is_empty() {
        return Err(String::from(
            "JUnit evidence did not contain a testsuite element",
        ));
    }
    let mut summary = summarize_suites(&document, &suite_indices)?;
    let testcase_indices = elements_named(&document, "testcase");
    append_testcase_evidence(&mut summary, &document, content, &testcase_indices)?;
    validate_summary(&mut summary, testcase_indices.len())?;
    Ok(summary)
}

fn elements_named(document: &XmlDocument, name: &str) -> Vec<usize> {
    document
        .elements
        .iter()
        .enumerate()
        .filter_map(|(index, element)| (element.name == name).then_some(index))
        .collect()
}

fn junit_count_attr(attrs: &str, name: &str) -> Result<u64, String> {
    let Some(value) = attr_string(attrs, name) else {
        return Ok(0);
    };
    value
        .parse::<u64>()
        .map_err(|_| format!("JUnit {name} attribute was not a valid non-negative integer"))
}

fn junit_duration_attr(attrs: &str) -> Result<Option<f64>, String> {
    let Some(value) = attr_string(attrs, "time") else {
        return Ok(None);
    };
    let seconds = value
        .parse::<f64>()
        .map_err(|_| String::from("JUnit duration was not numeric"))?;
    if !seconds.is_finite() || seconds < 0.0 {
        return Err(String::from(
            "JUnit duration was not a finite non-negative value",
        ));
    }
    Ok(Some(seconds))
}

fn junit_duration_ms(seconds: f64) -> Result<u64, String> {
    let milliseconds = seconds * 1000.0;
    if !milliseconds.is_finite() || milliseconds >= u64::MAX as f64 {
        return Err(String::from("JUnit duration exceeded the supported range"));
    }
    Ok(milliseconds as u64)
}

fn summarize_suites(document: &XmlDocument, suites: &[usize]) -> Result<JunitSummary, String> {
    let mut summary = JunitSummary::default();
    for &index in suites {
        let attrs = &document.elements[index].attrs;
        summary.tests =
            checked_count_add(summary.tests, junit_count_attr(attrs, "tests")?, "tests")?;
        summary.failures = checked_count_add(
            summary.failures,
            junit_count_attr(attrs, "failures")?,
            "failures",
        )?;
        summary.errors =
            checked_count_add(summary.errors, junit_count_attr(attrs, "errors")?, "errors")?;
        summary.skipped = checked_count_add(
            summary.skipped,
            junit_count_attr(attrs, "skipped")?,
            "skipped",
        )?;
        if let Some(seconds) = junit_duration_attr(attrs)? {
            summary.duration_ms = Some(checked_count_add(
                summary.duration_ms.unwrap_or(0),
                junit_duration_ms(seconds)?,
                "duration",
            )?);
        }
    }
    Ok(summary)
}

fn append_testcase_evidence(
    summary: &mut JunitSummary,
    document: &XmlDocument,
    content: &str,
    testcases: &[usize],
) -> Result<(), String> {
    let mut skipped = 0_u64;
    for &testcase in testcases {
        let attrs = &document.elements[testcase].attrs;
        let skipped_here = document
            .elements
            .iter()
            .enumerate()
            .any(|(index, element)| {
                element.name == "skipped" && document.has_ancestor_index(index, testcase)
            });
        if skipped_here {
            skipped = checked_count_add(skipped, 1, "skipped")?;
        }
        for (index, element) in document.elements.iter().enumerate() {
            if !matches!(element.name.as_str(), "failure" | "error")
                || !document.has_ancestor_index(index, testcase)
            {
                continue;
            }
            let message = attr_string(&element.attrs, "message").unwrap_or_else(|| {
                let text = document.text(content, element);
                if text.is_empty() {
                    String::from("JUnit test failed")
                } else {
                    text
                }
            });
            summary.offenders.push(TestFailure {
                file: attr_string(attrs, "classname"),
                line: None,
                message,
                test_name: attr_string(attrs, "name"),
            });
        }
    }
    summary.skipped = summary.skipped.max(skipped);
    Ok(())
}

fn validate_summary(summary: &mut JunitSummary, testcase_count: usize) -> Result<(), String> {
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
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{parse_junit_xml, parse_reports, test_row_passes, zero_tests_failure};
    use std::path::PathBuf;

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
    fn rejects_malformed_nested_junit_elements() {
        for content in [
            r#"<testsuite tests="1"><testcase><failure></error></testcase></testsuite>"#,
            r#"<testsuite tests="1"><testcase><error>broken</testcase></testsuite>"#,
            r#"<testsuite tests="1"><testcase><failure>broken</failure></testcase>"#,
        ] {
            assert!(parse_junit_xml(content).is_err(), "{content}");
        }
    }

    #[test]
    fn rejects_invalid_junit_numeric_attributes() {
        for content in [
            r#"<testsuite tests="bad"><testcase/></testsuite>"#,
            r#"<testsuite tests="1" failures="-1"><testcase/></testsuite>"#,
            r#"<testsuite tests="1" time="NaN"><testcase/></testsuite>"#,
            r#"<testsuite tests="1" time="1e308"><testcase/></testsuite>"#,
        ] {
            assert!(parse_junit_xml(content).is_err(), "{content}");
        }
    }

    #[test]
    fn missing_junit_reports_are_not_zero_tests() {
        assert_eq!(
            parse_reports(&[] as &[PathBuf]).err().as_deref(),
            Some("missing JUnit XML evidence")
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
