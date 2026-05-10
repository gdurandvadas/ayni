use ayni_core::{
    Budget, CommandFailure, Offenders, RunContext, Scope, SignalKind, SignalResult, SignalRow,
    TestFailure, TestResult,
};
use serde_json::json;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;

const STDERR_TAIL_LINES: usize = 20;

/// Run `cargo test` and stream each stdout/stderr line through `on_line`.
pub fn collect_with_lines<F>(context: &RunContext, mut on_line: F) -> Result<SignalRow, String>
where
    F: FnMut(&str),
{
    let (program, args) = test_command(context);
    let runner = format_command(&program, &args);
    let mut command = Command::new(&program);
    command.args(args.iter().map(String::as_str));
    command.current_dir(&context.execution.exec_cwd);
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    let mut child = command
        .spawn()
        .map_err(|error| format!("failed to execute {runner}: {error}"))?;

    enum PipeLine {
        Stdout(String),
        Stderr(String),
    }

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| String::from("cargo test stdout missing"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| String::from("cargo test stderr missing"))?;

    let (tx, rx) = mpsc::channel::<PipeLine>();
    let tx_out = tx.clone();
    let stdout_thread = thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines().map_while(Result::ok) {
            if tx_out.send(PipeLine::Stdout(line)).is_err() {
                break;
            }
        }
    });
    let stderr_thread = thread::spawn(move || {
        let reader = BufReader::new(stderr);
        for line in reader.lines().map_while(Result::ok) {
            if tx.send(PipeLine::Stderr(line)).is_err() {
                break;
            }
        }
    });

    let mut stdout_text = String::new();
    let mut stderr_text = String::new();
    for msg in rx {
        match msg {
            PipeLine::Stdout(line) => {
                on_line(&line);
                stdout_text.push_str(&line);
                stdout_text.push('\n');
            }
            PipeLine::Stderr(line) => {
                on_line(&line);
                stderr_text.push_str(&line);
                stderr_text.push('\n');
            }
        }
    }
    let _ = stdout_thread.join();
    let _ = stderr_thread.join();

    let success = child.wait().map(|s| s.success()).unwrap_or(false);
    if context.debug {
        eprintln!(
            "[debug] runner={} source={} kind={} resolved_from={} confidence={} ambiguous={}",
            context.execution.runner,
            context.execution.source,
            context.execution.kind,
            context.execution.resolved_from.display(),
            context.execution.confidence,
            context.execution.ambiguous
        );
        eprintln!(
            "[debug] cwd={} command={}",
            context.execution.exec_cwd.display(),
            runner
        );
        eprintln!("[debug] exit={}", if success { 0 } else { -1 });
    }
    Ok(build_test_row(
        context,
        success,
        &stdout_text,
        &stderr_text,
        &runner,
    ))
}

pub fn collect(context: &RunContext) -> Result<SignalRow, String> {
    collect_with_lines(context, |_| {})
}

fn build_test_row(
    context: &RunContext,
    success: bool,
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
                exit_code: None,
                message: stderr_tail(stderr, STDERR_TAIL_LINES),
            }),
        }),
        budget: Budget::Test(json!({})),
        offenders: Offenders::Test(offenders),
        delta_vs_previous: None,
        delta_vs_baseline: None,
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

fn format_command(program: &str, args: &[String]) -> String {
    if args.is_empty() {
        program.to_string()
    } else {
        format!("{program} {}", args.join(" "))
    }
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
}
