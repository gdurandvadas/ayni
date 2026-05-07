use super::util::{command_for_override_or_default, ensure_ayni_dir, run_command};
use ayni_core::{
    Budget, Language, Offenders, RunContext, Scope, SignalKind, SignalResult, SignalRow,
    TestFailure, TestResult,
};
use serde::Deserialize;
use serde_json::json;
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

pub fn collect(context: &RunContext) -> Result<SignalRow, String> {
    let artifact_dir = ensure_ayni_dir(context)?;
    let report_path = artifact_dir.join("pytest-report.json");
    let report_arg = format!("--json-report-file={}", report_path.display());
    let default_args = ["--json-report", report_arg.as_str()];
    let (program, args) =
        command_for_override_or_default(context, SignalKind::Test, "pytest", &default_args);
    let runner = format_command(&program, &args);
    let output = run_command(&context.workdir, &program, &args)?;
    let success = output.status.success();

    let report = read_report(&report_path).map_err(|error| {
        if success {
            error
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            format!("{error}; stderr: {}", stderr.trim())
        }
    })?;

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
    let offenders = report
        .tests
        .unwrap_or_default()
        .into_iter()
        .filter(|case| matches!(case.outcome.as_deref(), Some("failed" | "error")))
        .map(test_failure)
        .collect::<Vec<_>>();

    Ok(SignalRow {
        kind: SignalKind::Test,
        language: Language::Python,
        scope: Scope {
            workspace_root: context.scope.workspace_root.clone(),
            path: context.scope.path.clone(),
            package: context.scope.package.clone(),
            file: context.scope.file.clone(),
        },
        pass: success && failed == 0,
        result: SignalResult::Test(TestResult {
            total_tests,
            passed,
            failed,
            duration_ms,
            runner,
        }),
        budget: Budget::Test(json!({})),
        offenders: Offenders::Test(offenders),
        delta_vs_previous: None,
        delta_vs_baseline: None,
    })
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

fn format_command(program: &str, args: &[String]) -> String {
    if args.is_empty() {
        program.to_string()
    } else {
        format!("{program} {}", args.join(" "))
    }
}
