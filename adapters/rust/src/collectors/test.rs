use ayni_adapters_common::exec::{format_command, run_command_for_context_streaming};
use ayni_core::{
    Budget, CommandFailure, Offenders, RunContext, Scope, SignalKind, SignalResult, SignalRow,
    TestFailure, TestResult, TestSelection,
};
use serde_json::json;

const STDERR_TAIL_LINES: usize = 20;

/// Run `cargo test` and stream each stdout line through `on_line`.
pub fn collect_with_lines<F>(context: &RunContext, on_line: F) -> Result<SignalRow, String>
where
    F: FnMut(&str),
{
    let (program, args) = test_command(context);
    let runner = format_command(&program, &args);
    let output = run_command_for_context_streaming(context, &program, &args, on_line)?;
    let success = output.status.success();
    let stdout_text = String::from_utf8_lossy(&output.stdout);
    let stderr_text = String::from_utf8_lossy(&output.stderr);
    Ok(build_test_row(
        context,
        success,
        output.status.code(),
        &stdout_text,
        &stderr_text,
        &runner,
    ))
}

pub fn collect_selected_with_lines<F>(
    context: &RunContext,
    selection: &TestSelection,
    on_line: F,
) -> Result<SignalRow, String>
where
    F: FnMut(&str),
{
    if context.scope.file.is_some() {
        return Err(String::from(
            "Rust source-file selection is unsupported; use --package and optional --name",
        ));
    }
    let (program, args) = selected_test_command(context, selection)?;
    let runner = format_command(&program, &args);
    let output = run_command_for_context_streaming(context, &program, &args, on_line)?;
    Ok(build_test_row(
        context,
        output.status.success(),
        output.status.code(),
        &String::from_utf8_lossy(&output.stdout),
        &String::from_utf8_lossy(&output.stderr),
        &runner,
    ))
}

fn selected_test_command(
    context: &RunContext,
    selection: &TestSelection,
) -> Result<(String, Vec<String>), String> {
    if context.scope.file.is_some() {
        return Err(String::from(
            "Rust source-file selection is unsupported; use --package and optional --name",
        ));
    }
    let (program, mut args) = test_command(context);
    if let Some(package) = &context.scope.package {
        args.push(String::from("--package"));
        args.push(package.clone());
    }
    if let Some(name) = &selection.name {
        args.push(name.clone());
    }
    Ok((program, args))
}

pub fn collect(context: &RunContext) -> Result<SignalRow, String> {
    collect_with_lines(context, |_| {})
}

fn build_test_row(
    context: &RunContext,
    success: bool,
    exit_code: Option<i32>,
    stdout: &str,
    stderr: &str,
    runner: &str,
) -> SignalRow {
    let parsed = parse_all_test_result_lines(stdout);
    let (total_tests, passed, failed) = parsed.unwrap_or((0, 0, 0));

    let mut offenders = Vec::new();
    if !success {
        offenders.push(TestFailure {
            file: None,
            line: None,
            message: format!(
                "cargo test failed; stderr tail:\n{}",
                stderr_tail(stderr, STDERR_TAIL_LINES)
            ),
            test_name: None,
        });
    }

    SignalRow {
        kind: SignalKind::Test,
        language: ayni_core::Language::Rust,
        scope: Scope {
            workspace_root: context.scope.workspace_root.clone(),
            path: context.scope.path.clone(),
            package: context.scope.package.clone(),
            file: context.scope.file.clone(),
        },
        pass: failed == 0 && success,
        result: SignalResult::Test(TestResult {
            total_tests,
            passed,
            failed,
            duration_ms: None,
            runner: runner.to_string(),
            failure: (!success).then(|| CommandFailure {
                category: String::from("repo_code_issue"),
                classification: String::from("command_error"),
                command: runner.to_string(),
                cwd: context.execution.exec_cwd.display().to_string(),
                exit_code,
                message: stderr_tail(stderr, STDERR_TAIL_LINES),
            }),
        }),
        budget: Budget::Test(json!({})),
        offenders: Offenders::Test(offenders),
        delta_vs_previous: None,
    }
}

fn test_command(context: &RunContext) -> (String, Vec<String>) {
    if let Some(override_cmd) = context
        .policy
        .tool_override_for(ayni_core::Language::Rust, SignalKind::Test)
    {
        let args = if override_cmd.args.is_empty() {
            vec![String::from("test")]
        } else {
            override_cmd.args.clone()
        };
        return (override_cmd.command.clone(), args);
    }
    (String::from("cargo"), vec![String::from("test")])
}

/// Sum every `test result:` line (`cargo test` emits one per crate / phase).
fn parse_all_test_result_lines(stdout: &str) -> Option<(u64, u64, u64)> {
    let mut passed_sum = 0_u64;
    let mut failed_sum = 0_u64;
    let mut any = false;
    for line in stdout.lines() {
        if line.trim_start().starts_with("test result:")
            && let Some((_total, passed, failed)) = parse_single_test_result_line(line)
        {
            any = true;
            passed_sum = passed_sum.saturating_add(passed);
            failed_sum = failed_sum.saturating_add(failed);
        }
    }
    any.then(|| {
        (
            passed_sum.saturating_add(failed_sum),
            passed_sum,
            failed_sum,
        )
    })
}

fn parse_single_test_result_line(line: &str) -> Option<(u64, u64, u64)> {
    let mut passed: Option<u64> = None;
    let mut failed: Option<u64> = None;
    for segment in line.split([';', ',']) {
        let trimmed = segment.trim();
        if let Some(value) = parse_count_segment(trimmed, "passed") {
            passed = Some(value);
        }
        if let Some(value) = parse_count_segment(trimmed, "failed") {
            failed = Some(value);
        }
    }
    let passed = passed?;
    let failed = failed?;
    Some((passed.saturating_add(failed), passed, failed))
}

fn parse_count_segment(segment: &str, suffix: &str) -> Option<u64> {
    let value_text = segment.strip_suffix(suffix)?.trim();
    value_text
        .split_whitespace()
        .last()
        .and_then(|token| token.parse::<u64>().ok())
}

fn stderr_tail(stderr: &str, line_count: usize) -> String {
    let lines: Vec<&str> = stderr.lines().collect();
    let start = lines.len().saturating_sub(line_count);
    lines[start..].join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
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
            execution: ExecutionResolution::direct("cargo", PathBuf::from("."), "test", 100),
            debug: false,
        }
    }

    #[test]
    fn aggregates_multiple_test_result_lines() {
        let stdout = "\
Running tests crate A
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n\
Running tests crate B
test result: ok. 20 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n\
Doc-tests\n\
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n\
";
        let parsed = parse_all_test_result_lines(stdout).expect("aggregate");
        assert_eq!(parsed.1, 25);
        assert_eq!(parsed.2, 0);
        assert_eq!(parsed.0, 25);
    }

    #[test]
    fn default_test_command_is_cargo_test() {
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
enabled = ["rust"]
"#,
        );
        let (program, args) = test_command(&context);
        assert_eq!(program, "cargo");
        assert_eq!(args, vec!["test"]);
    }

    #[test]
    fn focused_command_maps_package_and_name_to_cargo() {
        let mut context = context_with_policy(
            r#"
[checks]
test = true
[languages]
enabled = ["rust"]
"#,
        );
        context.scope.package = Some(String::from("ayni-core"));
        let (_, args) = selected_test_command(
            &context,
            &TestSelection {
                language: ayni_core::Language::Rust,
                name: Some(String::from("parses")),
            },
        )
        .expect("selected command");
        assert_eq!(args, ["test", "--package", "ayni-core", "parses"]);
    }

    #[test]
    fn focused_command_rejects_rust_source_file() {
        let mut context = context_with_policy(
            r#"
[checks]
test = true
[languages]
enabled = ["rust"]
"#,
        );
        context.scope.file = Some(String::from("core/src/lib.rs"));
        let error = selected_test_command(
            &context,
            &TestSelection {
                language: ayni_core::Language::Rust,
                name: None,
            },
        )
        .expect_err("file rejected");
        assert!(error.contains("source-file selection is unsupported"));
    }

    #[test]
    fn test_command_uses_rust_tooling_override() {
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
enabled = ["rust"]

[rust.tooling.test]
command = "cargo"
args = ["nextest", "run"]
"#,
        );
        let (program, args) = test_command(&context);
        assert_eq!(program, "cargo");
        assert_eq!(args, vec!["nextest", "run"]);
    }

    #[test]
    fn failed_test_row_preserves_command_exit_code() {
        let row = build_test_row(
            &context_with_policy(
                r#"
[checks]
test = true
coverage = false
size = false
complexity = false
deps = false
mutation = false

[languages]
enabled = ["rust"]
"#,
            ),
            false,
            Some(17),
            "",
            "forced failure",
            "cargo test",
        );

        let SignalResult::Test(result) = row.result else {
            panic!("test result");
        };
        assert_eq!(result.failure.expect("failure").exit_code, Some(17));
    }
}
