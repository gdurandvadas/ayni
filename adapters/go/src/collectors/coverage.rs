use super::util::to_repo_relative_path;
use super::util::{command_failure_from_output, run_tool_for_context};
use ayni_core::{
    Budget, CoverageOffender, CoveragePolicy, CoverageResult, Language, Level, Offenders,
    RunContext, Scope, SignalKind, SignalResult, SignalRow,
};
use serde_json::json;
use std::fs;
use std::process::Command;

pub fn collect(context: &RunContext) -> Result<SignalRow, String> {
    let profile_path = context.workdir.join(".ayni-go-cover.out");
    let profile_arg = format!("-coverprofile={}", profile_path.display());
    let (test_program, test_args, test_engine) = coverage_test_command(context, &profile_arg);
    let test_output = run_tool_for_context(context, &test_program, &test_args)
        .map_err(|error| format!("failed to execute {test_engine}: {error}"))?;

    let (status, percent, line_percent) = if test_output.status.success() {
        let cover_output = Command::new("go")
            .arg("tool")
            .arg("cover")
            .arg("-func")
            .arg(&profile_path)
            .current_dir(&context.workdir)
            .output()
            .map_err(|error| format!("failed to execute go tool cover: {error}"))?;
        if cover_output.status.success() {
            let text = String::from_utf8_lossy(&cover_output.stdout);
            let line = parse_total_percent(&text);
            (String::from("ok"), line, line)
        } else {
            (String::from("error"), None, None)
        }
    } else {
        (String::from("error"), None, None)
    };

    let _ = fs::remove_file(&profile_path);

    let coverage_config = context.policy.go.coverage.as_ref();
    let coverage_budget = coverage_config
        .map(|config| {
            json!({
                "line_percent_warn": config.line_percent.map(|v| v.warn),
                "line_percent_fail": config.line_percent.map(|v| v.fail),
            })
        })
        .unwrap_or_else(|| json!({}));

    let pass = status == "ok"
        && coverage_config
            .and_then(|c| c.line_percent)
            .is_none_or(|t| percent.is_none_or(|v| v >= t.fail));

    let offenders = build_offenders(percent, coverage_config, context);

    Ok(SignalRow {
        kind: SignalKind::Coverage,
        language: Language::Go,
        scope: Scope {
            workspace_root: context.scope.workspace_root.clone(),
            path: context.scope.path.clone(),
            package: context.scope.package.clone(),
            file: context.scope.file.clone(),
        },
        pass,
        result: SignalResult::Coverage(CoverageResult {
            percent,
            line_percent,
            branch_percent: None,
            engine: format!("{test_engine} + go tool cover"),
            status,
            failure: (!test_output.status.success()).then(|| {
                command_failure_from_output(
                    context,
                    SignalKind::Coverage,
                    &test_program,
                    &test_args,
                    &test_output,
                )
            }),
        }),
        budget: Budget::Coverage(coverage_budget),
        offenders: Offenders::Coverage(offenders),
        delta_vs_previous: None,
        delta_vs_baseline: None,
    })
}

fn coverage_test_command(context: &RunContext, profile_arg: &str) -> (String, Vec<String>, String) {
    if let Some(override_cmd) = context
        .policy
        .tool_override_for(Language::Go, SignalKind::Coverage)
    {
        let mut args = if override_cmd.args.is_empty() {
            vec![String::from("test"), String::from("./...")]
        } else {
            override_cmd.args.clone()
        };
        if !args.iter().any(|arg| arg.starts_with("-coverprofile=")) {
            args.push(profile_arg.to_string());
        }
        let engine = format_command(&override_cmd.command, &args);
        return (override_cmd.command.clone(), args, engine);
    }
    let args = vec![
        String::from("test"),
        String::from("./..."),
        profile_arg.to_string(),
    ];
    let engine = format_command("go", &args);
    (String::from("go"), args, engine)
}

fn format_command(program: &str, args: &[String]) -> String {
    if args.is_empty() {
        program.to_string()
    } else {
        format!("{program} {}", args.join(" "))
    }
}

fn parse_total_percent(text: &str) -> Option<f64> {
    for line in text.lines().rev() {
        let trimmed = line.trim();
        if !trimmed.starts_with("total:") {
            continue;
        }
        let token = trimmed
            .split_whitespace()
            .last()
            .map(|value| value.trim_end_matches('%'));
        if let Some(token) = token
            && let Ok(value) = token.parse::<f64>()
        {
            return Some(value);
        }
    }
    None
}

fn build_offenders(
    headline: Option<f64>,
    policy: Option<&CoveragePolicy>,
    context: &RunContext,
) -> Vec<CoverageOffender> {
    let Some(value) = headline else {
        return Vec::new();
    };
    let Some(threshold) = policy.and_then(|p| p.line_percent) else {
        return Vec::new();
    };
    if value >= threshold.warn {
        return Vec::new();
    }
    let level = if value < threshold.fail {
        Level::Fail
    } else {
        Level::Warn
    };
    vec![CoverageOffender {
        file: to_repo_relative_path(&context.repo_root, &context.workdir),
        line: None,
        value,
        level,
    }]
}

#[cfg(test)]
mod tests {
    use super::{coverage_test_command, parse_total_percent};
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
    fn parses_total_percent_line() {
        let output = "pkg/a.go:10:\tfoo\t66.7%\ntotal:\t(statements)\t83.3%\n";
        assert_eq!(parse_total_percent(output), Some(83.3));
    }

    #[test]
    fn default_coverage_command_appends_cover_profile() {
        let context = context_with_policy(
            r#"
[checks]
test = false
coverage = true
size = false
complexity = false
deps = false
mutation = false

[languages]
enabled = ["go"]
"#,
        );
        let (_, args, _) = coverage_test_command(&context, "-coverprofile=.ayni-go-cover.out");
        assert!(
            args.iter()
                .any(|arg| arg == "-coverprofile=.ayni-go-cover.out")
        );
    }

    #[test]
    fn coverage_command_uses_go_tooling_override() {
        let context = context_with_policy(
            r#"
[checks]
test = false
coverage = true
size = false
complexity = false
deps = false
mutation = false

[languages]
enabled = ["go"]

[go.tooling.coverage]
command = "go"
args = ["test", "./...", "-run", "TestFast"]
"#,
        );
        let (program, args, engine) =
            coverage_test_command(&context, "-coverprofile=.ayni-go-cover.out");
        assert_eq!(program, "go");
        assert!(
            args.iter()
                .any(|arg| arg == "-coverprofile=.ayni-go-cover.out")
        );
        assert!(engine.starts_with("go test ./..."));
    }
}
