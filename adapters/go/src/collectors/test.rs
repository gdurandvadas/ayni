use super::util::{command_failure_from_output, run_tool_for_context};
use ayni_core::{
    Budget, Language, Offenders, RunContext, Scope, SignalKind, SignalResult, SignalRow,
    TestFailure, TestResult,
};
use serde::Deserialize;
use serde_json::json;

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

pub fn collect(context: &RunContext) -> Result<SignalRow, String> {
    let (program, args) = test_command(context);
    let runner = format_command(&program, &args);
    let output = run_tool_for_context(context, &program, &args)?;
    let success = output.status.success();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    let mut offenders = Vec::new();
    let mut total_tests = 0_u64;
    let mut passed = 0_u64;
    let mut failed = 0_u64;
    let mut duration_ms = 0_u64;

    for line in stdout.lines() {
        let Ok(event) = serde_json::from_str::<GoTestEvent>(line) else {
            continue;
        };
        let Some(action) = event.action.as_deref() else {
            continue;
        };
        if event.test.is_some() {
            match action {
                "pass" => {
                    total_tests += 1;
                    passed += 1;
                }
                "fail" => {
                    total_tests += 1;
                    failed += 1;
                    offenders.push(TestFailure {
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
                duration_ms = duration_ms.saturating_add((elapsed * 1000.0) as u64);
            }
        } else if action == "output"
            && let Some(out) = event.output
            && out.contains("FAIL")
        {
            offenders.push(TestFailure {
                file: event.package.clone(),
                line: None,
                message: out.trim().to_string(),
                test_name: None,
            });
        }
    }

    if !success && offenders.is_empty() {
        offenders.push(TestFailure {
            file: None,
            line: None,
            message: stderr.trim().to_string(),
            test_name: None,
        });
    }

    Ok(SignalRow {
        kind: SignalKind::Test,
        language: Language::Go,
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
            duration_ms: (duration_ms > 0).then_some(duration_ms),
            runner,
            failure: (!success).then(|| {
                command_failure_from_output(context, SignalKind::Test, &program, &args, &output)
            }),
        }),
        budget: Budget::Test(json!({})),
        offenders: Offenders::Test(offenders),
        delta_vs_previous: None,
        delta_vs_baseline: None,
    })
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

fn format_command(program: &str, args: &[String]) -> String {
    if args.is_empty() {
        program.to_string()
    } else {
        format!("{program} {}", args.join(" "))
    }
}

#[cfg(test)]
mod tests {
    use super::test_command;
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
            execution: ExecutionResolution::direct("go", PathBuf::from("."), "test", 100),
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
}
