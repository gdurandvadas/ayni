use super::util::{package_manager_for_context, run_command, run_tool};
use ayni_core::{
    Budget, Offenders, RunContext, Scope, SignalKind, SignalResult, SignalRow, TestFailure,
    TestResult,
};
use serde_json::Value as JsonValue;
use serde_json::json;

pub fn collect(context: &RunContext) -> Result<SignalRow, String> {
    let (output, runner) = if let Some((program, args, runner)) = test_override_command(context) {
        (run_command(&context.workdir, &program, &args)?, runner)
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

    let pass = status_ok && failed == 0;
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
        }),
        budget: Budget::Test(json!({})),
        offenders: Offenders::Test(offenders),
        delta_vs_previous: None,
        delta_vs_baseline: None,
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

fn format_command(program: &str, args: &[String]) -> String {
    if args.is_empty() {
        program.to_string()
    } else {
        format!("{program} {}", args.join(" "))
    }
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
    use super::test_override_command;
    use ayni_core::{AyniPolicy, RunContext, Scope};
    use std::path::PathBuf;

    fn context_with_policy(document: &str) -> RunContext {
        let policy: AyniPolicy = toml::from_str(document).expect("policy");
        RunContext {
            repo_root: PathBuf::from("."),
            workdir: PathBuf::from("."),
            policy,
            scope: Scope::default(),
            diff: None,
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
}
