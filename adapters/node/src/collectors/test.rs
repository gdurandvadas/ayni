use super::util::{command_failure_from_output, run_tool, tool_command};
use ayni_adapters_common::collector::{CollectorError, CollectorResult};
use ayni_adapters_common::exec::{
    format_command, run_command_for_context_streaming_structured,
    run_command_for_context_structured,
};
use ayni_adapters_common::failure::{setup_failure, test_execution_incomplete};
use ayni_core::{
    Budget, Offenders, RunContext, Scope, SignalKind, SignalResult, SignalRow, TestBudget,
    TestFailure, TestResult, VerificationSelection,
};
use serde_json::Value as JsonValue;

pub fn collect(context: &RunContext) -> CollectorResult {
    let (output, runner) = if let Some((program, args, runner)) = test_override_command(context) {
        (
            run_command_for_context_structured(context, &program, &args)?,
            runner,
        )
    } else {
        let (program, args) = tool_command(
            context,
            "vitest",
            &["run", "--reporter=json", "--passWithNoTests"],
        );
        let runner = format_command(&program, &args);
        (
            run_tool(
                context,
                "vitest",
                &["run", "--reporter=json", "--passWithNoTests"],
            )?,
            runner,
        )
    };
    let status_ok = output.status.success();
    let stdout_text = String::from_utf8_lossy(&output.stdout);
    let stderr_text = String::from_utf8_lossy(&output.stderr);
    let mut summary = normalize_vitest_output(&stdout_text, &stderr_text);

    if summary.report_missing && !status_ok {
        summary.offenders.push(TestFailure {
            file: None,
            line: None,
            message: stderr_text.trim().to_string(),
            test_name: None,
        });
    }

    if status_ok && !summary.report_missing && summary.total_tests == 0 {
        summary.offenders.push(zero_tests_failure());
    }

    let pass = test_row_passes(
        status_ok,
        summary.total_tests,
        summary.failed,
        summary.report_missing,
    );
    let execution_incomplete = summary.report_missing
        || test_execution_incomplete(status_ok, summary.total_tests, summary.failed);
    let failure = if execution_incomplete && !status_ok {
        Some(command_failure_from_output(
            context,
            SignalKind::Test,
            runner.split_whitespace().next().unwrap_or("node"),
            &runner
                .split_whitespace()
                .skip(1)
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            &output,
        ))
    } else if summary.report_missing {
        Some(setup_failure(
            context,
            runner.clone(),
            "test runner exited successfully but produced no parseable JSON report; \
             cannot verify test results (check the reporter configuration)",
        ))
    } else {
        None
    };
    Ok(SignalRow {
        kind: SignalKind::Test,
        language: ayni_core::Language::Node,
        scope: Scope {
            workspace_root: context.scope.workspace_root.clone(),
            path: context.scope.path.clone(),
            package: context.scope.package.clone(),
            file: context.scope.file.clone(),
        },
        pass,
        result: SignalResult::Test(TestResult {
            total_tests: summary.total_tests,
            passed: summary.passed,
            failed: summary.failed,
            duration_ms: summary.duration_ms,
            runner,
            failure,
        }),
        budget: Budget::Test(TestBudget::default()),
        offenders: Offenders::Test(summary.offenders),
    })
}

pub fn collect_selected(
    context: &RunContext,
    selection: &VerificationSelection,
    on_line: &mut dyn FnMut(&str),
) -> CollectorResult {
    let (program, mut args) = selected_test_command(context).map_err(CollectorError::Adapter)?;
    if let Some(file) = &context.scope.file {
        args.push(selected_file_argument(context, file));
    }
    if let Some(name) = &selection.name {
        args.push(String::from("--testNamePattern"));
        args.push(name.clone());
    }
    let runner = format_command(&program, &args);
    let output = run_command_for_context_streaming_structured(context, &program, &args, on_line)?;
    build_row_from_output(context, output, runner)
}

fn selected_file_argument(context: &RunContext, file: &str) -> String {
    let Some(root) = context.scope.path.as_deref() else {
        return file.to_string();
    };
    if let Some(relative) = file.strip_prefix(&format!("{root}/")) {
        return relative.to_string();
    }

    let parents = root
        .split('/')
        .filter(|component| !component.is_empty() && *component != ".")
        .count();
    format!("{}{file}", "../".repeat(parents))
}

fn selected_test_command(context: &RunContext) -> Result<(String, Vec<String>), String> {
    let (program, mut args, _) = test_override_command(context).unwrap_or_else(|| {
        let (program, args) = tool_command(
            context,
            "vitest",
            &["run", "--reporter=json", "--passWithNoTests"],
        );
        let runner = format_command(&program, &args);
        (program, args, runner)
    });
    if let Some(package) = &context.scope.package {
        match program.as_str() {
            "pnpm" | "bun" => args.splice(0..0, [String::from("--filter"), package.clone()]),
            "npm" => args.splice(0..0, [String::from("--workspace"), package.clone()]),
            "yarn" => args.splice(0..0, [String::from("workspace"), package.clone()]),
            _ => {
                return Err(format!(
                    "Node package selection is unsupported for custom runner {program}"
                ));
            }
        };
    }
    Ok((program, args))
}

fn build_row_from_output(
    context: &RunContext,
    output: std::process::Output,
    runner: String,
) -> CollectorResult {
    let status_ok = output.status.success();
    let stdout_text = String::from_utf8_lossy(&output.stdout);
    let stderr_text = String::from_utf8_lossy(&output.stderr);
    let mut summary = normalize_vitest_output(&stdout_text, &stderr_text);
    if status_ok && !summary.report_missing && summary.total_tests == 0 {
        summary.offenders.push(zero_tests_failure());
    }
    let execution_incomplete = summary.report_missing
        || test_execution_incomplete(status_ok, summary.total_tests, summary.failed);
    let failure = if execution_incomplete && !status_ok {
        Some(command_failure_from_output(
            context,
            SignalKind::Test,
            &runner,
            &[],
            &output,
        ))
    } else if summary.report_missing {
        Some(setup_failure(
            context,
            runner.clone(),
            "test runner produced no parseable JSON report",
        ))
    } else {
        None
    };
    Ok(SignalRow {
        kind: SignalKind::Test,
        language: ayni_core::Language::Node,
        scope: context.scope.clone(),
        pass: test_row_passes(
            status_ok,
            summary.total_tests,
            summary.failed,
            summary.report_missing,
        ),
        result: SignalResult::Test(TestResult {
            total_tests: summary.total_tests,
            passed: summary.passed,
            failed: summary.failed,
            duration_ms: None,
            runner,
            failure,
        }),
        budget: Budget::Test(TestBudget::default()),
        offenders: Offenders::Test(summary.offenders),
    })
}

struct VitestSummary {
    report_missing: bool,
    total_tests: u64,
    passed: u64,
    failed: u64,
    duration_ms: Option<u64>,
    offenders: Vec<TestFailure>,
}

fn normalize_vitest_output(stdout: &str, stderr: &str) -> VitestSummary {
    let Some(report) = parse_vitest_report(stdout).or_else(|| parse_vitest_report(stderr)) else {
        return missing_vitest_summary();
    };
    let Some((total_tests, passed, failed)) = valid_vitest_counts(&report) else {
        return missing_vitest_summary();
    };

    VitestSummary {
        report_missing: false,
        total_tests,
        passed,
        failed,
        duration_ms: report
            .get("testResults")
            .and_then(JsonValue::as_array)
            .map(|results| {
                results
                    .iter()
                    .filter_map(|item| item.get("endTime").and_then(JsonValue::as_u64))
                    .sum::<u64>()
            })
            .filter(|value| *value > 0),
        offenders: extract_failures(&report),
    }
}

fn missing_vitest_summary() -> VitestSummary {
    VitestSummary {
        report_missing: true,
        total_tests: 0,
        passed: 0,
        failed: 0,
        duration_ms: None,
        offenders: Vec::new(),
    }
}

fn valid_vitest_counts(report: &JsonValue) -> Option<(u64, u64, u64)> {
    let total = report.get("numTotalTests").and_then(JsonValue::as_u64)?;
    let passed = report.get("numPassedTests").and_then(JsonValue::as_u64)?;
    let failed = report.get("numFailedTests").and_then(JsonValue::as_u64)?;
    (passed.checked_add(failed)? <= total).then_some((total, passed, failed))
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

fn test_row_passes(status_ok: bool, total_tests: u64, failed: u64, report_missing: bool) -> bool {
    status_ok && total_tests > 0 && failed == 0 && !report_missing
}

fn test_override_command(context: &RunContext) -> Option<(String, Vec<String>, String)> {
    let override_cmd = context
        .policy
        .tool_override_for(ayni_core::Language::Node, SignalKind::Test)?;
    let args = if override_cmd.args.is_empty() {
        vec![
            String::from("run"),
            String::from("--reporter=json"),
            String::from("--passWithNoTests"),
        ]
    } else {
        override_cmd.args.clone()
    };
    let runner = format_command(&override_cmd.command, &args);
    Some((override_cmd.command.clone(), args, runner))
}

fn parse_vitest_report(raw: &str) -> Option<JsonValue> {
    // Vitest JSON reporter may be mixed with log lines; extract the last JSON object.
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(value) = serde_json::from_str::<JsonValue>(trimmed) {
        return Some(value);
    }
    let start = trimmed.rfind('\n').map(|idx| idx + 1).unwrap_or(0);
    serde_json::from_str::<JsonValue>(&trimmed[start..]).ok()
}

fn extract_failures(report: &JsonValue) -> Vec<TestFailure> {
    let mut failures = Vec::new();
    let Some(suites) = report.get("testResults").and_then(JsonValue::as_array) else {
        return failures;
    };
    for suite in suites {
        let file = suite
            .get("name")
            .and_then(JsonValue::as_str)
            .map(String::from);
        let Some(assertions) = suite.get("assertionResults").and_then(JsonValue::as_array) else {
            continue;
        };
        for assertion in assertions {
            if assertion.get("status").and_then(JsonValue::as_str) != Some("failed") {
                continue;
            }
            let message = assertion
                .get("failureMessages")
                .and_then(JsonValue::as_array)
                .and_then(|messages| messages.first())
                .and_then(JsonValue::as_str)
                .or_else(|| assertion.get("failureMessage").and_then(JsonValue::as_str))
                .unwrap_or("test failed")
                .to_string();
            let test_name = assertion
                .get("fullName")
                .and_then(JsonValue::as_str)
                .or_else(|| assertion.get("title").and_then(JsonValue::as_str))
                .map(String::from);
            failures.push(TestFailure {
                file: file.clone(),
                line: None,
                message,
                test_name,
            });
        }
    }
    failures
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use super::build_row_from_output;
    use super::{
        selected_file_argument, selected_test_command, test_override_command, test_row_passes,
        zero_tests_failure,
    };
    use ayni_core::{AyniPolicy, ExecutionResolution, RunContext, Scope};
    #[cfg(unix)]
    use ayni_core::{Offenders, SignalResult};
    #[cfg(unix)]
    use std::os::unix::process::ExitStatusExt;
    use std::path::PathBuf;
    #[cfg(unix)]
    use std::process::{ExitStatus, Output};

    fn context_with_policy(document: &str) -> RunContext {
        let policy: AyniPolicy = toml::from_str(document).expect("policy");
        RunContext {
            repo_root: PathBuf::from("."),
            target_root: PathBuf::from("."),
            workdir: PathBuf::from("."),
            policy,
            scope: Scope::default(),
            execution: ExecutionResolution::direct("npm", PathBuf::from("."), "test", 100),
            cancellation: Default::default(),
            debug: false,
        }
    }

    #[test]
    fn no_override_returns_none() {
        let context = context_with_policy(
            r#"
[checks]
test = true
coverage = false
size = false
complexity = false
deps = false
mutation = false

[languages]
enabled = ["node"]
"#,
        );
        assert!(test_override_command(&context).is_none());
    }

    #[test]
    fn test_override_command_uses_node_tooling_override() {
        let context = context_with_policy(
            r#"
[checks]
test = true
coverage = false
size = false
complexity = false
deps = false
mutation = false

[languages]
enabled = ["node"]

[node.tooling.test]
command = "pnpm"
args = ["exec", "vitest", "run"]
"#,
        );
        let (program, args, runner) =
            test_override_command(&context).expect("expected node test override");
        assert_eq!(program, "pnpm");
        assert_eq!(args, vec!["exec", "vitest", "run"]);
        assert_eq!(runner, "pnpm exec vitest run");
    }

    #[test]
    fn focused_command_inserts_pnpm_workspace_filter() {
        let mut context = context_with_policy(
            r#"
[checks]
test = true
[languages]
enabled = ["node"]
[node.tooling.test]
command = "pnpm"
args = ["exec", "vitest", "run", "--reporter=json"]
"#,
        );
        context.scope.package = Some(String::from("@guita/web"));
        let (program, args) = selected_test_command(&context).expect("selected command");
        assert_eq!(program, "pnpm");
        assert_eq!(&args[..4], ["--filter", "@guita/web", "exec", "vitest"]);
    }

    #[test]
    fn focused_file_is_relative_to_the_node_execution_root() {
        let mut context = context_with_policy(
            r#"
[checks]
test = true
[languages]
enabled = ["node"]
"#,
        );
        context.scope.path = Some(String::from("frontend"));
        assert_eq!(
            selected_file_argument(&context, "tests/dev-stack.test.mjs"),
            "../tests/dev-stack.test.mjs"
        );
        assert_eq!(
            selected_file_argument(&context, "frontend/apps/web/src/money.test.ts"),
            "apps/web/src/money.test.ts"
        );
    }

    #[test]
    fn zero_test_finding_is_actionable() {
        assert!(!test_row_passes(true, 0, 0, false));
        assert!(
            zero_tests_failure()
                .message
                .contains("discovered zero tests")
        );
    }

    #[cfg(unix)]
    #[test]
    fn successful_vitest_zero_test_report_fails_without_a_command_failure() {
        let context =
            context_with_policy("[checks]\ntest = true\n[languages]\nenabled = [\"node\"]");
        let output = Output {
            status: ExitStatus::from_raw(0),
            stdout: br#"{"numTotalTests":0,"numPassedTests":0,"numFailedTests":0}"#.to_vec(),
            stderr: Vec::new(),
        };
        let row =
            build_row_from_output(&context, output, String::from("vitest run")).expect("test row");

        assert!(!row.pass);
        let SignalResult::Test(result) = &row.result else {
            panic!("test result");
        };
        assert_eq!(result.total_tests, 0);
        assert!(result.failure.is_none());
        let Offenders::Test(offenders) = &row.offenders else {
            panic!("test offenders");
        };
        assert!(offenders[0].message.contains("discovered zero tests"));
    }

    #[cfg(unix)]
    #[test]
    fn vitest_startup_failure_without_report_emits_valid_command_failure_row() {
        let context =
            context_with_policy("[checks]\ntest = true\n[languages]\nenabled = [\"node\"]");
        let output = Output {
            status: ExitStatus::from_raw(1 << 8),
            stdout: Vec::new(),
            stderr: b"Error: Cannot find module '@sveltejs/vite-plugin-svelte'".to_vec(),
        };
        let row =
            build_row_from_output(&context, output, String::from("vitest run")).expect("test row");

        assert!(!row.pass);
        let SignalResult::Test(result) = &row.result else {
            panic!("test result");
        };
        assert_eq!(result.total_tests, 0);
        assert_eq!(result.passed, 0);
        assert_eq!(result.failed, 0);
        let failure = result.failure.as_ref().expect("command failure");
        assert_eq!(failure.classification, "import_error");
        assert!(failure.message.contains("@sveltejs/vite-plugin-svelte"));
        row.validate_payloads().expect("valid failed test row");
    }

    #[cfg(unix)]
    #[test]
    fn invalid_vitest_reports_emit_valid_incomplete_evidence() {
        let context =
            context_with_policy("[checks]\ntest = true\n[languages]\nenabled = [\"node\"]");
        for report in [
            r#"{"numFailedTests":1}"#,
            r#"{"numTotalTests":0,"numPassedTests":0,"numFailedTests":1}"#,
        ] {
            let output = Output {
                status: ExitStatus::from_raw(1 << 8),
                stdout: report.as_bytes().to_vec(),
                stderr: b"Vitest exited before completing its report".to_vec(),
            };
            let row = build_row_from_output(&context, output, String::from("vitest run"))
                .expect("test row");

            assert!(!row.pass);
            let SignalResult::Test(result) = &row.result else {
                panic!("test result");
            };
            assert_eq!(result.total_tests, 0);
            assert_eq!(result.passed, 0);
            assert_eq!(result.failed, 0);
            assert!(result.failure.is_some());
            row.validate_payloads().expect("valid incomplete test row");
        }
    }
}
