use super::util::{
    attr_f64, attr_string, attr_u64, command_failure_from_output, format_command, gradle_command,
    run_command_for_context,
};
use ayni_core::{
    Budget, Language, Offenders, RunContext, Scope, SignalKind, SignalResult, SignalRow,
    TestFailure, TestResult,
};
use regex::Regex;
use serde_json::json;
use std::fs;
use std::path::Path;
use walkdir::WalkDir;

pub fn collect(context: &RunContext) -> Result<SignalRow, String> {
    let (program, args) = gradle_command(context, SignalKind::Test, "test");
    let runner = format_command(&program, &args);
    let output = run_command_for_context(context, &program, &args)?;
    let report = parse_reports(&context.workdir.join("build/test-results/test"))?;
    let failed = report.failures + report.errors;

    Ok(SignalRow {
        kind: SignalKind::Test,
        language: Language::Kotlin,
        scope: Scope {
            workspace_root: context.scope.workspace_root.clone(),
            path: context.scope.path.clone(),
            package: context.scope.package.clone(),
            file: context.scope.file.clone(),
        },
        pass: output.status.success() && failed == 0,
        result: SignalResult::Test(TestResult {
            total_tests: report.tests,
            passed: report
                .tests
                .saturating_sub(failed)
                .saturating_sub(report.skipped),
            failed,
            duration_ms: report.duration_ms,
            runner,
            failure: (!output.status.success()).then(|| {
                command_failure_from_output(context, SignalKind::Test, &program, &args, &output)
            }),
        }),
        budget: Budget::Test(json!({})),
        offenders: Offenders::Test(report.offenders),
        delta_vs_previous: None,
        delta_vs_baseline: None,
    })
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

fn parse_reports(dir: &Path) -> Result<JunitSummary, String> {
    if !dir.exists() {
        return Ok(JunitSummary::default());
    }
    let mut summary = JunitSummary::default();
    for entry in WalkDir::new(dir).into_iter().filter_map(Result::ok) {
        if !entry.file_type().is_file()
            || entry.path().extension().and_then(|value| value.to_str()) != Some("xml")
        {
            continue;
        }
        let content = fs::read_to_string(entry.path())
            .map_err(|error| format!("failed to read {}: {error}", entry.path().display()))?;
        let parsed = parse_junit_xml(&content)?;
        summary.tests += parsed.tests;
        summary.failures += parsed.failures;
        summary.errors += parsed.errors;
        summary.skipped += parsed.skipped;
        summary.duration_ms =
            Some(summary.duration_ms.unwrap_or(0) + parsed.duration_ms.unwrap_or(0));
        summary.offenders.extend(parsed.offenders);
    }
    if summary.duration_ms == Some(0) {
        summary.duration_ms = None;
    }
    Ok(summary)
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
    let mut summary = JunitSummary::default();
    for caps in testsuite_re.captures_iter(content) {
        let attrs = caps.get(1).map(|value| value.as_str()).unwrap_or("");
        summary.tests += attr_u64(attrs, "tests").unwrap_or(0);
        summary.failures += attr_u64(attrs, "failures").unwrap_or(0);
        summary.errors += attr_u64(attrs, "errors").unwrap_or(0);
        summary.skipped += attr_u64(attrs, "skipped").unwrap_or(0);
        if let Some(seconds) = attr_f64(attrs, "time") {
            summary.duration_ms =
                Some(summary.duration_ms.unwrap_or(0) + (seconds * 1000.0) as u64);
        }
    }
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
            summary.skipped += 1;
        }
    }
    if summary.tests == 0 {
        summary.tests = testcase_re.captures_iter(content).count() as u64;
    }
    if summary.failures + summary.errors == 0 && !summary.offenders.is_empty() {
        summary.failures = summary.offenders.len() as u64;
    }
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::parse_junit_xml;

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
}
