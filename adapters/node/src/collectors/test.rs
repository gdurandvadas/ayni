use super::util::{command_failure_from_output, package_manager_for_context, run_tool};
use ayni_adapters_common::exec::{
    format_command, run_command_for_context, run_command_for_context_streaming,
};
use ayni_adapters_common::failure::setup_failure;
use ayni_core::{
    Budget, Offenders, RunContext, Scope, SignalKind, SignalResult, SignalRow, TestFailure,
    TestResult, TestSelection,
};
use serde_json::Value as JsonValue;
use serde_json::json;

pub fn collect(context: &RunContext) -> Result<SignalRow, String> {
    let (output, runner) = if let Some((program, args, runner)) = test_override_command(context) {
        (run_command_for_context(context, &program, &args)?, runner)
    } else {
        let output = run_tool(
            context,
            "vitest",
            &["run", "--reporter=json", "--passWithNoTests"],
        )?;
        let manager = package_manager_for_context(context);
        (output, format!("{} exec vitest", manager.executable()))
    };
    let status_ok = output.status.success();
    let stdout_text = String::from_utf8_lossy(&output.stdout);
    let stderr_text = String::from_utf8_lossy(&output.stderr);
    let report = parse_vitest_report(&stdout_text).or_else(|| parse_vitest_report(&stderr_text));
    let report_missing = report.is_none();

    let mut total_tests = 0_u64;
    let mut passed = 0_u64;
    let mut failed = if status_ok { 0_u64 } else { 1_u64 };
    let mut duration_ms = None::<u64>;
    let mut offenders = Vec::<TestFailure>::new();

    if let Some(report) = report {
        total_tests = report
            .get("numTotalTests")
            .and_then(JsonValue::as_u64)
            .unwrap_or(0);
        passed = report
            .get("numPassedTests")
            .and_then(JsonValue::as_u64)
            .unwrap_or(0);
        failed = report
            .get("numFailedTests")
            .and_then(JsonValue::as_u64)
            .unwrap_or(failed);
        duration_ms = report
            .get("testResults")
            .and_then(JsonValue::as_array)
            .map(|results| {
                results
                    .iter()
                    .filter_map(|item| item.get("endTime").and_then(JsonValue::as_u64))
                    .sum::<u64>()
            })
            .filter(|value| *value > 0);
        offenders = extract_failures(&report);
    } else if !status_ok {
        offenders.push(TestFailure {
            file: None,
            line: None,
            message: stderr_text.trim().to_string(),
            test_name: None,
        });
    }

    let pass = status_ok && failed == 0 && !report_missing;
    let failure = if !status_ok {
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
    } else if report_missing {
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
            total_tests,
            passed,
            failed,
            duration_ms,
            runner,
            failure,
        }),
        budget: Budget::Test(json!({})),
        offenders: Offenders::Test(offenders),
        delta_vs_previous: None,
    })
}

pub fn collect_selected(
    context: &RunContext,
    selection: &TestSelection,
    on_line: &mut dyn FnMut(&str),
) -> Result<SignalRow, String> {
    let (program, mut args) = selected_test_command(context)?;
    if let Some(file) = &context.scope.file {
        args.push(selected_file_argument(context, file));
    }
    if let Some(name) = &selection.name {
        args.push(String::from("--testNamePattern"));
        args.push(name.clone());
    }
    let runner = format_command(&program, &args);
    let output = run_command_for_context_streaming(context, &program, &args, on_line)?;
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
        let manager = package_manager_for_context(context);
        let (program, args) =
            manager.exec_command("vitest", &["run", "--reporter=json", "--passWithNoTests"]);
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
) -> Result<SignalRow, String> {
    let status_ok = output.status.success();
    let stdout_text = String::from_utf8_lossy(&output.stdout);
    let stderr_text = String::from_utf8_lossy(&output.stderr);
    let report = parse_vitest_report(&stdout_text).or_else(|| parse_vitest_report(&stderr_text));
    let report_missing = report.is_none();
    let mut total_tests = 0;
    let mut passed = 0;
    let mut failed = u64::from(!status_ok);
    let mut offenders = Vec::new();
    if let Some(report) = report {
        total_tests = report
            .get("numTotalTests")
            .and_then(JsonValue::as_u64)
            .unwrap_or(0);
        passed = report
            .get("numPassedTests")
            .and_then(JsonValue::as_u64)
            .unwrap_or(0);
        failed = report
            .get("numFailedTests")
            .and_then(JsonValue::as_u64)
            .unwrap_or(failed);
        offenders = extract_failures(&report);
    }
    let failure = if !status_ok {
        Some(command_failure_from_output(
            context,
            SignalKind::Test,
            &runner,
            &[],
            &output,
        ))
    } else if report_missing {
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
        pass: status_ok && failed == 0 && !report_missing,
        result: SignalResult::Test(TestResult {
            total_tests,
            passed,
            failed,
            duration_ms: None,
            runner,
            failure,
        }),
        budget: Budget::Test(json!({})),
        offenders: Offenders::Test(offenders),
        delta_vs_previous: None,
    })
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
    use super::{selected_file_argument, selected_test_command, test_override_command};
    use ayni_core::{AyniPolicy, ExecutionResolution, RunContext, Scope};
    use std::path::PathBuf;

    fn context_with_policy(document: &str) -> RunContext {
        let policy: AyniPolicy = toml::from_str(document).expect("policy");
        RunContext {
            repo_root: PathBuf::from("."),
            target_root: PathBuf::from("."),
            workdir: PathBuf::from("."),
            policy,
            scope: Scope::default(),
            diff: None,
            execution: ExecutionResolution::direct("npm", PathBuf::from("."), "test", 100),
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
}
