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
use std::collections::BTreeMap;

#[derive(Debug, Deserialize)]
struct GoTestEvent {
    #[serde(rename = "Action")]
    action: Option<String>,
    #[serde(rename = "Package")]
    package: Option<String>,
    #[serde(rename = "ImportPath")]
    import_path: Option<String>,
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

fn go_package_target_for_file(context: &RunContext, file: &str) -> String {
    let file = if std::path::Path::new(file).is_absolute() {
        std::path::PathBuf::from(file)
    } else {
        context.repo_root.join(file)
    };
    let package = file.parent().unwrap_or(&context.execution.exec_cwd);
    let target = package
        .strip_prefix(&context.execution.exec_cwd)
        .unwrap_or(package);
    if target.as_os_str().is_empty() {
        String::from(".")
    } else if target.is_absolute() {
        target.to_string_lossy().into_owned()
    } else {
        format!("./{}", target.to_string_lossy().replace('\\', "/"))
    }
}

pub fn collect_selected(
    context: &RunContext,
    selection: &VerificationSelection,
    _on_line: &mut dyn FnMut(&str),
) -> CollectorResult {
    let (program, mut args) = test_command(context);
    let target = context
        .scope
        .file
        .as_deref()
        .map(|file| go_package_target_for_file(context, file))
        .or_else(|| context.scope.package.clone());
    if let Some(target) = target {
        if let Some(default_target) = args.iter_mut().find(|arg| *arg == "./...") {
            *default_target = target;
        } else {
            args.push(target);
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
    package_runs: BTreeMap<String, u64>,
    test_runs: BTreeMap<(String, String), u64>,
    terminals: BTreeMap<(String, Option<String>), Vec<String>>,
    terminal_errors: Vec<String>,
}

impl TestSummary {
    fn evidence_error(&self) -> Option<String> {
        if self.malformed_events > 0 {
            return Some(format!(
                "{} non-empty output line(s) were not valid Go test JSON events",
                self.malformed_events
            ));
        }
        if !self.terminal_errors.is_empty() {
            return Some(self.terminal_errors.join("; "));
        }
        let unterminated_packages = self
            .package_runs
            .iter()
            .filter_map(|(package, runs)| {
                let terminals = self
                    .terminals
                    .get(&(package.clone(), None))
                    .map_or(0, Vec::len) as u64;
                (terminals < *runs).then(|| package.clone())
            })
            .collect::<Vec<_>>();
        let unterminated_tests = self
            .test_runs
            .iter()
            .filter_map(|((package, test), runs)| {
                let terminals = self
                    .terminals
                    .get(&(package.clone(), Some(test.clone())))
                    .map_or(0, Vec::len) as u64;
                (terminals < *runs).then(|| format!("{package}:{test}"))
            })
            .collect::<Vec<_>>();
        if !unterminated_packages.is_empty() || !unterminated_tests.is_empty() {
            let mut missing = Vec::new();
            if !unterminated_packages.is_empty() {
                missing.push(format!("packages {}", unterminated_packages.join(", ")));
            }
            if !unterminated_tests.is_empty() {
                missing.push(format!("tests {}", unterminated_tests.join(", ")));
            }
            return Some(format!(
                "terminal events were missing for {}",
                missing.join("; ")
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
        let has_subject = match event.action.as_deref() {
            Some("build-output" | "build-fail") => event.import_path.is_some(),
            Some(_) => event.package.is_some(),
            None => false,
        };
        if !has_subject {
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
    if matches!(action, "build-output" | "build-fail") {
        let Some(import_path) = event.import_path.as_deref() else {
            return;
        };
        record_build_event(summary, action, import_path, event.output.as_deref());
        return;
    }
    let Some(package) = event.package.as_ref() else {
        return;
    };
    let test = event.test.as_ref();
    record_lifecycle_event(summary, action, package, test, event.elapsed);
    record_package_output_failure(summary, action, package, test, event.output.as_deref());
}

fn record_build_event(
    summary: &mut TestSummary,
    action: &str,
    import_path: &str,
    output: Option<&str>,
) {
    let message = match (
        action,
        output.map(str::trim).filter(|output| !output.is_empty()),
    ) {
        ("build-output", Some(output)) => Some(output.to_string()),
        ("build-fail", _) => Some(format!("package '{import_path}' failed to build")),
        _ => None,
    };
    if let Some(message) = message {
        summary.offenders.push(TestFailure {
            file: Some(import_path.to_string()),
            line: None,
            message,
            test_name: None,
        });
    }
}

fn record_lifecycle_event(
    summary: &mut TestSummary,
    action: &str,
    package: &str,
    test: Option<&String>,
    elapsed: Option<f64>,
) {
    if let Some(test) = test
        && !package_is_open(summary, package)
    {
        let ordering = if summary.package_runs.contains_key(package) {
            "after package completion"
        } else {
            "before package start"
        };
        summary.terminal_errors.push(format!(
            "test event '{action}' appeared {ordering} for {package}:{test}"
        ));
        return;
    }
    match (action, test) {
        ("start", None) => {
            *summary.package_runs.entry(package.to_string()).or_default() += 1;
        }
        ("run", Some(test)) => {
            *summary
                .test_runs
                .entry((package.to_string(), test.clone()))
                .or_default() += 1;
        }
        ("pass" | "fail" | "skip", _) => {
            record_terminal_event(summary, action, package, test, elapsed);
        }
        _ => {}
    }
}

fn package_is_open(summary: &TestSummary, package: &str) -> bool {
    let runs = summary.package_runs.get(package).copied().unwrap_or(0);
    let terminals = summary
        .terminals
        .get(&(package.to_string(), None))
        .map_or(0, Vec::len) as u64;
    runs > terminals
}

fn record_terminal_event(
    summary: &mut TestSummary,
    action: &str,
    package: &str,
    test: Option<&String>,
    elapsed: Option<f64>,
) {
    if test.is_none() {
        let open_tests = summary
            .test_runs
            .iter()
            .any(|((test_package, test), runs)| {
                test_package == package
                    && (summary
                        .terminals
                        .get(&(test_package.clone(), Some(test.clone())))
                        .map_or(0, Vec::len) as u64)
                        < *runs
            });
        if open_tests {
            summary.terminal_errors.push(format!(
                "package terminal '{action}' appeared before all tests completed for {package}"
            ));
        }
    }
    let key = (package.to_string(), test.cloned());
    let run_count = expected_run_count(summary, package, test);
    let terminal_count = summary.terminals.get(&key).map_or(0, Vec::len) as u64;
    if terminal_count >= run_count {
        record_unmatched_terminal(summary, action, package, test, run_count);
        return;
    }
    summary
        .terminals
        .entry(key)
        .or_default()
        .push(action.to_string());
    if let Some(test) = test {
        record_completed_test(summary, action, package, test, elapsed);
    }
}

fn expected_run_count(summary: &TestSummary, package: &str, test: Option<&String>) -> u64 {
    match test {
        Some(test) => summary
            .test_runs
            .get(&(package.to_string(), test.clone()))
            .copied()
            .unwrap_or(0),
        None => summary.package_runs.get(package).copied().unwrap_or(0),
    }
}

fn record_unmatched_terminal(
    summary: &mut TestSummary,
    action: &str,
    package: &str,
    test: Option<&String>,
    run_count: u64,
) {
    let subject = test.map_or_else(|| package.to_string(), |test| format!("{package}:{test}"));
    let description = if run_count == 0 {
        "without a corresponding run"
    } else {
        "as a duplicate terminal or conflicting terminal"
    };
    summary.terminal_errors.push(format!(
        "terminal '{action}' event appeared {description} for {subject}"
    ));
}

fn record_completed_test(
    summary: &mut TestSummary,
    action: &str,
    package: &str,
    test: &str,
    elapsed: Option<f64>,
) {
    summary.total_tests = summary.total_tests.saturating_add(1);
    match action {
        "pass" => summary.passed = summary.passed.saturating_add(1),
        "fail" => {
            summary.failed = summary.failed.saturating_add(1);
            summary.offenders.push(TestFailure {
                file: Some(package.to_string()),
                line: None,
                message: format!("test '{test}' failed"),
                test_name: Some(test.to_string()),
            });
        }
        "skip" => {}
        _ => unreachable!("terminal actions are filtered above"),
    }
    if let Some(elapsed) = elapsed {
        summary.duration_ms = summary
            .duration_ms
            .saturating_add((elapsed * 1000.0) as u64);
    }
}

fn record_package_output_failure(
    summary: &mut TestSummary,
    action: &str,
    package: &str,
    test: Option<&String>,
    output: Option<&str>,
) {
    if test.is_none()
        && action == "output"
        && let Some(output) = output
        && output.contains("FAIL")
    {
        summary.offenders.push(TestFailure {
            file: Some(package.to_string()),
            line: None,
            message: output.trim().to_string(),
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
    use super::{
        go_package_target_for_file, parse_test_events, test_command, test_row_passes,
        zero_tests_failure,
    };
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
    fn file_selection_targets_the_containing_go_package() {
        let mut context = context_with_policy("");
        context.repo_root = PathBuf::from("repo");
        context.execution.exec_cwd = PathBuf::from("repo");
        assert_eq!(
            go_package_target_for_file(&context, "internal/api/api_test.go"),
            "./internal/api"
        );
        context.execution.exec_cwd = PathBuf::from("repo/internal/api");
        assert_eq!(
            go_package_target_for_file(&context, "internal/api/api_test.go"),
            "."
        );
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
    fn rejects_malformed_missing_and_terminal_without_run_events() {
        let malformed =
            parse_test_events("{\"Action\":\"start\",\"Package\":\"example.com/a\"}\n{");
        assert!(malformed.evidence_error().unwrap().contains("not valid"));

        let unterminated = parse_test_events(
            "{\"Action\":\"start\",\"Package\":\"example.com/a\"}\n{\"Action\":\"run\",\"Package\":\"example.com/a\",\"Test\":\"TestA\"}\n",
        );
        assert!(
            unterminated
                .evidence_error()
                .unwrap()
                .contains("terminal events were missing")
        );

        let terminal_without_run = parse_test_events(
            "{\"Action\":\"start\",\"Package\":\"example.com/a\"}\n{\"Action\":\"pass\",\"Package\":\"example.com/a\",\"Test\":\"TestA\"}\n",
        );
        assert!(
            terminal_without_run
                .evidence_error()
                .unwrap()
                .contains("without a corresponding run")
        );
    }

    #[test]
    fn accepts_nested_subtests_and_rejects_duplicate_or_conflicting_terminals() {
        let nested = parse_test_events(
            "{\"Action\":\"start\",\"Package\":\"example.com/a\"}\n{\"Action\":\"run\",\"Package\":\"example.com/a\",\"Test\":\"TestParent\"}\n{\"Action\":\"run\",\"Package\":\"example.com/a\",\"Test\":\"TestParent/child\"}\n{\"Action\":\"skip\",\"Package\":\"example.com/a\",\"Test\":\"TestParent/child\"}\n{\"Action\":\"pass\",\"Package\":\"example.com/a\",\"Test\":\"TestParent\"}\n{\"Action\":\"pass\",\"Package\":\"example.com/a\"}\n",
        );
        assert!(nested.evidence_error().is_none());
        assert_eq!(
            (nested.total_tests, nested.passed, nested.failed),
            (2, 1, 0)
        );

        let repeated_run = parse_test_events(
            "{\"Action\":\"start\",\"Package\":\"example.com/a\"}\n{\"Action\":\"run\",\"Package\":\"example.com/a\",\"Test\":\"TestA\"}\n{\"Action\":\"run\",\"Package\":\"example.com/a\",\"Test\":\"TestA\"}\n{\"Action\":\"pass\",\"Package\":\"example.com/a\",\"Test\":\"TestA\"}\n",
        );
        assert!(
            repeated_run
                .evidence_error()
                .unwrap()
                .contains("terminal events were missing")
        );

        let duplicate = parse_test_events(
            "{\"Action\":\"start\",\"Package\":\"example.com/a\"}\n{\"Action\":\"run\",\"Package\":\"example.com/a\",\"Test\":\"TestA\"}\n{\"Action\":\"pass\",\"Package\":\"example.com/a\",\"Test\":\"TestA\"}\n{\"Action\":\"fail\",\"Package\":\"example.com/a\",\"Test\":\"TestA\"}\n{\"Action\":\"pass\",\"Package\":\"example.com/a\"}\n",
        );
        assert!(
            duplicate
                .evidence_error()
                .unwrap()
                .contains("duplicate terminal")
        );
    }

    #[test]
    fn rejects_test_events_without_an_open_package() {
        let summary = parse_test_events(
            "{\"Action\":\"run\",\"Package\":\"example.com/a\",\"Test\":\"TestA\"}\n{\"Action\":\"pass\",\"Package\":\"example.com/a\",\"Test\":\"TestA\"}\n",
        );
        assert!(
            summary
                .evidence_error()
                .expect("ordering error")
                .contains("before package start")
        );
    }

    #[test]
    fn rejects_package_completion_before_test_completion() {
        let summary = parse_test_events(
            "{\"Action\":\"start\",\"Package\":\"example.com/a\"}\n{\"Action\":\"run\",\"Package\":\"example.com/a\",\"Test\":\"TestA\"}\n{\"Action\":\"pass\",\"Package\":\"example.com/a\"}\n{\"Action\":\"pass\",\"Package\":\"example.com/a\",\"Test\":\"TestA\"}\n",
        );
        let error = summary.evidence_error().expect("ordering error");
        assert!(error.contains("before all tests completed"));
        assert!(error.contains("after package completion"));
    }

    #[test]
    fn accepts_build_events_with_import_paths() {
        let summary = parse_test_events(
            "{\"ImportPath\":\"example.com/a\",\"Action\":\"build-output\",\"Output\":\"./broken.go:1: undefined: missing\\n\"}\n{\"ImportPath\":\"example.com/a\",\"Action\":\"build-fail\"}\n",
        );
        assert!(summary.evidence_error().is_none());
        assert_eq!(summary.offenders.len(), 2);
        assert_eq!(summary.offenders[0].file.as_deref(), Some("example.com/a"));
        assert!(summary.offenders[1].message.contains("failed to build"));
    }

    #[test]
    fn successful_zero_test_run_fails_with_an_actionable_finding() {
        let failure = zero_tests_failure();
        assert!(failure.message.contains("discovered zero tests"));
        assert!(!test_row_passes(true, 0, 0));
    }
}
