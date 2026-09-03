use super::util::run_tool_for_context;
use ayni_adapters_common::collector::CollectorResult;
use ayni_adapters_common::exec::format_command;
use ayni_adapters_common::failure::{
    command_failure_from_output, setup_failure, test_execution_incomplete,
};
use ayni_core::{
    Budget, Language, Offenders, RunContext, SignalKind, SignalResult, SignalRow, TestBudget,
    TestFailure, TestResult, VerificationSelection,
};
use serde::Deserialize;
use std::collections::BTreeSet;

#[derive(Debug, Deserialize)]
struct GoTestEvent {
    #[serde(rename = "Action")]
    action: Option<String>,
    #[serde(rename = "Package")]
    package: Option<String>,
    #[serde(rename = "Test")]
    test: Option<String>,
    #[serde(rename = "Elapsed")]
    elapsed: Option<f64>,
    #[serde(rename = "Output")]
    output: Option<String>,
}

pub fn collect(context: &RunContext) -> CollectorResult {
    let (program, args) = test_command(context);
    collect_with_command(context, program, args)
}

pub fn collect_selected(
    context: &RunContext,
    selection: &VerificationSelection,
    _on_line: &mut dyn FnMut(&str),
) -> CollectorResult {
    let (program, mut args) = test_command(context);
    if let Some(target) = context
        .scope
        .file
        .as_ref()
        .or(context.scope.package.as_ref())
    {
        if let Some(default_target) = args.iter_mut().find(|arg| *arg == "./...") {
            *default_target = target.clone();
        } else {
            args.push(target.clone());
        }
    }
    if let Some(name) = &selection.name {
        args.extend([String::from("-run"), format!("^{name}$")]);
    }
    collect_with_command(context, program, args)
}

fn collect_with_command(
    context: &RunContext,
    program: String,
    args: Vec<String>,
) -> CollectorResult {
    let runner = format_command(&program, &args);
    let output = run_tool_for_context(context, &program, &args)?;
    Ok(build_row_from_output(
        context, &program, &args, &output, runner,
    ))
}

pub(super) fn build_row_from_output(
    context: &RunContext,
    program: &str,
    args: &[String],
    output: &std::process::Output,
    runner: String,
) -> SignalRow {
    let success = output.status.success();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let mut summary = parse_test_events(&stdout);

    let evidence_failure = summary.evidence_error().map(|message| {
        setup_failure(
            context,
            runner.clone(),
            format!("go test JSON evidence was incomplete: {message}"),
        )
    });
    let evidence_complete = evidence_failure.is_none();
    let failure = evidence_failure.or_else(|| {
        test_execution_incomplete(success, summary.total_tests, summary.failed)
            .then(|| command_failure_from_output(context, SignalKind::Test, program, args, output))
    });

    if !success && summary.offenders.is_empty() {
        summary.offenders.push(TestFailure {
            file: None,
            line: None,
            message: stderr.trim().to_string(),
            test_name: None,
        });
    } else if success && summary.total_tests == 0 {
        summary.offenders.push(zero_tests_failure());
    }

    SignalRow {
        kind: SignalKind::Test,
        language: Language::Go,
        scope: context.scope.clone(),
        pass: evidence_complete && test_row_passes(success, summary.total_tests, summary.failed),
        result: SignalResult::Test(TestResult {
            total_tests: summary.total_tests,
            passed: summary.passed,
            failed: summary.failed,
            duration_ms: (summary.duration_ms > 0).then_some(summary.duration_ms),
            runner,
            failure,
        }),
        budget: Budget::Test(TestBudget::default()),
        offenders: Offenders::Test(summary.offenders),
    }
}

#[derive(Default)]
struct TestSummary {
    offenders: Vec<TestFailure>,
    total_tests: u64,
    passed: u64,
    failed: u64,
    duration_ms: u64,
    malformed_events: u64,
    packages_with_tests: BTreeSet<String>,
    terminal_packages: BTreeSet<String>,
}

impl TestSummary {
    fn evidence_error(&self) -> Option<String> {
        if self.malformed_events > 0 {
            return Some(format!(
                "{} non-empty output line(s) were not valid Go test JSON events",
                self.malformed_events
            ));
        }
        let unterminated = self
            .packages_with_tests
            .difference(&self.terminal_packages)
            .cloned()
            .collect::<Vec<_>>();
        if !unterminated.is_empty() {
            return Some(format!(
                "package-level terminal events were missing for {}",
                unterminated.join(", ")
            ));
        }
        None
    }
}

fn parse_test_events(stdout: &str) -> TestSummary {
    let mut summary = TestSummary::default();
    for line in stdout.lines().filter(|line| !line.trim().is_empty()) {
        let Ok(event) = serde_json::from_str::<GoTestEvent>(line) else {
            summary.malformed_events = summary.malformed_events.saturating_add(1);
            continue;
        };
        if event.action.is_none() || event.package.is_none() {
            summary.malformed_events = summary.malformed_events.saturating_add(1);
            continue;
        }
        record_test_event(&mut summary, event);
    }
    summary
}

fn record_test_event(summary: &mut TestSummary, event: GoTestEvent) {
    let Some(action) = event.action.as_deref() else {
        return;
    };
    if let Some(package) = &event.package {
        if event.test.is_some() {
            summary.packages_with_tests.insert(package.clone());
        } else if matches!(action, "pass" | "fail" | "skip") {
            summary.terminal_packages.insert(package.clone());
        }
    }
    if event.test.is_some() {
        match action {
            "pass" => {
                summary.total_tests += 1;
                summary.passed += 1;
            }
            "fail" => {
                summary.total_tests += 1;
                summary.failed += 1;
                summary.offenders.push(TestFailure {
                    file: event.package.clone(),
                    line: None,
                    message: format!(
                        "test '{}' failed",
                        event.test.as_deref().unwrap_or("<unknown>")
                    ),
                    test_name: event.test.clone(),
                });
            }
            _ => {}
        }
        if let Some(elapsed) = event.elapsed {
            summary.duration_ms = summary
                .duration_ms
                .saturating_add((elapsed * 1000.0) as u64);
        }
    } else if action == "output"
        && let Some(out) = event.output
        && out.contains("FAIL")
    {
        summary.offenders.push(TestFailure {
            file: event.package.clone(),
            line: None,
            message: out.trim().to_string(),
            test_name: None,
        });
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

fn test_command(context: &RunContext) -> (String, Vec<String>) {
    if let Some(override_cmd) = context
        .policy
        .tool_override_for(Language::Go, SignalKind::Test)
    {
        let args = if override_cmd.args.is_empty() {
            vec![
                String::from("test"),
                String::from("./..."),
                String::from("-json"),
            ]
        } else {
            override_cmd.args.clone()
        };
        return (override_cmd.command.clone(), args);
    }
    (
        String::from("go"),
        vec![
            String::from("test"),
            String::from("./..."),
            String::from("-json"),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::{parse_test_events, test_command, test_row_passes, zero_tests_failure};
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
            execution: ExecutionResolution::direct("go", PathBuf::from("."), "test", 100),
            cancellation: Default::default(),
            debug: false,
        }
    }

    #[test]
    fn default_test_command_is_go_test_json() {
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
enabled = ["go"]
"#,
        );
        let (program, args) = test_command(&context);
        assert_eq!(program, "go");
        assert_eq!(args, vec!["test", "./...", "-json"]);
    }

    #[test]
    fn test_command_uses_go_tooling_override() {
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
enabled = ["go"]

[go.tooling.test]
command = "gotestsum"
args = ["--jsonfile", ".ayni/go-tests.json", "--", "./..."]
"#,
        );
        let (program, args) = test_command(&context);
        assert_eq!(program, "gotestsum");
        assert_eq!(
            args,
            vec!["--jsonfile", ".ayni/go-tests.json", "--", "./..."]
        );
    }

    #[test]
    fn rejects_malformed_or_unterminated_go_test_json_streams() {
        let malformed = parse_test_events(
            "{\"Action\":\"pass\",\"Package\":\"example.com/a\",\"Test\":\"TestA\"}\n{",
        );
        assert!(malformed.evidence_error().unwrap().contains("not valid"));

        let unterminated = parse_test_events(
            "{\"Action\":\"pass\",\"Package\":\"example.com/a\",\"Test\":\"TestA\"}\n",
        );
        assert!(
            unterminated
                .evidence_error()
                .unwrap()
                .contains("terminal events were missing")
        );

        let complete = parse_test_events(
            "{\"Action\":\"pass\",\"Package\":\"example.com/a\",\"Test\":\"TestA\"}\n{\"Action\":\"pass\",\"Package\":\"example.com/a\"}\n",
        );
        assert!(complete.evidence_error().is_none());
    }

    #[test]
    fn successful_zero_test_run_fails_with_an_actionable_finding() {
        let failure = zero_tests_failure();
        assert!(failure.message.contains("discovered zero tests"));
        assert!(!test_row_passes(true, 0, 0));
    }
}
