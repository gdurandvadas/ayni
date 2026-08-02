use super::util::run_tool_for_context;
use ayni_adapters_common::collector::{CollectorError, CollectorResult};
use ayni_adapters_common::exec::format_command;
use ayni_adapters_common::exec::run_command_for_context_structured;
use ayni_adapters_common::failure::{command_failure_from_output, coverage_metric_failure};
use ayni_adapters_common::paths::to_repo_relative_path;
use ayni_adapters_common::reports::prepare_report_path;
use ayni_core::{
    Budget, ConfiguredMetricEvaluation, CoverageOffender, CoveragePolicy, CoverageResult, Language,
    Level, Offenders, RunContext, Scope, SignalKind, SignalResult, SignalRow,
    evaluate_configured_metric,
};
use serde_json::json;
use std::fs;

pub fn collect(context: &RunContext) -> CollectorResult {
    let profile_path =
        prepare_report_path(context, "go", "coverage.out").map_err(CollectorError::Adapter)?;
    let profile_arg = format!("-coverprofile={}", profile_path.display());
    let (test_program, test_args, test_engine) = coverage_test_command(context, &profile_arg);
    let test_output = match run_tool_for_context(context, &test_program, &test_args) {
        Ok(output) => output,
        Err(error) => {
            let _ = fs::remove_file(&profile_path);
            return Err(error.into());
        }
    };

    let (status, raw_line_percent, cover_failure) = if test_output.status.success() {
        let cover_args = vec![
            String::from("tool"),
            String::from("cover"),
            String::from("-func"),
            profile_path.to_string_lossy().into_owned(),
        ];
        let cover_output = match run_command_for_context_structured(context, "go", &cover_args) {
            Ok(output) => output,
            Err(error) => {
                let _ = fs::remove_file(&profile_path);
                return Err(error.into());
            }
        };
        if cover_output.status.success() {
            let text = String::from_utf8_lossy(&cover_output.stdout);
            let line = parse_total_percent(&text);
            (String::from("ok"), line, None)
        } else {
            (
                String::from("error"),
                None,
                Some(command_failure_from_output(
                    context,
                    SignalKind::Coverage,
                    "go",
                    &cover_args,
                    &cover_output,
                )),
            )
        }
    } else {
        (String::from("error"), None, None)
    };

    let _ = fs::remove_file(&profile_path);
    let line_percent = finite_percent(raw_line_percent);
    let percent = line_percent;

    let coverage_config = context.policy.go.coverage.as_ref();
    let coverage_budget = coverage_config
        .map(|config| {
            json!({
                "line_percent_warn": config.line_percent.map(|v| v.warn),
                "line_percent_fail": config.line_percent.map(|v| v.fail),
                "branch_percent_warn": config.branch_percent.map(|v| v.warn),
                "branch_percent_fail": config.branch_percent.map(|v| v.fail),
            })
        })
        .unwrap_or_else(|| json!({}));

    let assessment = assess_coverage(raw_line_percent, coverage_config, context);
    let metric_failure = coverage_metric_failure(
        context,
        format!("{test_engine} + go tool cover"),
        "line_percent",
        assessment.line,
    )
    .or_else(|| {
        coverage_metric_failure(
            context,
            format!("{test_engine} + go tool cover"),
            "branch_percent",
            assessment.branch,
        )
    });
    let pass = status == "ok" && metric_failure.is_none() && !assessment.has_fail;

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
            failure: (!test_output.status.success())
                .then(|| {
                    command_failure_from_output(
                        context,
                        SignalKind::Coverage,
                        &test_program,
                        &test_args,
                        &test_output,
                    )
                })
                .or(cover_failure)
                .or(metric_failure),
        }),
        budget: Budget::Coverage(coverage_budget),
        offenders: Offenders::Coverage(assessment.offenders),
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
        if let Some(token) = token {
            return Some(token.parse::<f64>().unwrap_or(f64::NAN));
        }
    }
    None
}

struct CoverageAssessment {
    line: ConfiguredMetricEvaluation,
    branch: ConfiguredMetricEvaluation,
    offenders: Vec<CoverageOffender>,
    has_fail: bool,
}

fn assess_coverage(
    line_percent: Option<f64>,
    policy: Option<&CoveragePolicy>,
    context: &RunContext,
) -> CoverageAssessment {
    let line =
        evaluate_configured_metric(line_percent, policy.and_then(|policy| policy.line_percent));
    // Standard Go profiles expose statement coverage only; never reinterpret it
    // as branch coverage.
    let branch = evaluate_configured_metric(None, policy.and_then(|policy| policy.branch_percent));
    let mut offenders = Vec::new();
    if let ConfiguredMetricEvaluation::Present {
        value,
        level: Some(level),
    } = line
    {
        offenders.push(CoverageOffender {
            file: to_repo_relative_path(&context.repo_root, &context.workdir),
            line: None,
            value,
            level,
        });
    }
    let has_fail = offenders
        .iter()
        .any(|offender| offender.level == Level::Fail);
    CoverageAssessment {
        line,
        branch,
        offenders,
        has_fail,
    }
}

fn finite_percent(value: Option<f64>) -> Option<f64> {
    value.filter(|value| value.is_finite())
}

#[cfg(test)]
mod tests {
    use super::{assess_coverage, coverage_test_command, parse_total_percent};
    use ayni_core::{
        AyniPolicy, ConfiguredMetricEvaluation, CoveragePolicy, ExecutionResolution, Level,
        RunContext, Scope, ThresholdFloat,
    };
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
            debug: false,
        }
    }

    #[test]
    fn parses_total_percent_line() {
        let output = "pkg/a.go:10:\tfoo\t66.7%\ntotal:\t(statements)\t83.3%\n";
        assert_eq!(parse_total_percent(output), Some(83.3));
    }

    fn policy(warn: f64, fail: f64) -> CoveragePolicy {
        CoveragePolicy {
            line_percent: Some(ThresholdFloat { warn, fail }),
            branch_percent: None,
        }
    }

    #[test]
    fn enforces_line_threshold_boundaries_and_preserves_measured_zero() {
        let context = context_with_policy("");
        let policy = policy(80.0, 70.0);
        let equal_warn = assess_coverage(Some(80.0), Some(&policy), &context);
        assert!(equal_warn.offenders.is_empty());
        assert!(!equal_warn.has_fail);

        let equal_fail = assess_coverage(Some(70.0), Some(&policy), &context);
        assert_eq!(equal_fail.offenders[0].level, Level::Warn);
        assert!(!equal_fail.has_fail);

        let below_fail = assess_coverage(Some(69.0), Some(&policy), &context);
        assert_eq!(below_fail.offenders[0].level, Level::Fail);
        assert!(below_fail.has_fail);

        let zero = assess_coverage(Some(0.0), Some(&policy), &context);
        assert_eq!(zero.offenders[0].value, 0.0);
        assert!(zero.has_fail);
    }

    #[test]
    fn rejects_missing_or_malformed_line_evidence_and_configured_branches() {
        let context = context_with_policy("");
        let policy = policy(80.0, 70.0);
        assert!(matches!(
            assess_coverage(None, Some(&policy), &context).line,
            ConfiguredMetricEvaluation::Missing
        ));
        assert!(matches!(
            assess_coverage(Some(f64::NAN), Some(&policy), &context).line,
            ConfiguredMetricEvaluation::Unparseable
        ));
        assert!(
            parse_total_percent("total:\t(statements)\tunknown%\n")
                .expect("malformed total is represented")
                .is_nan()
        );

        let branch_only = CoveragePolicy {
            line_percent: None,
            branch_percent: Some(ThresholdFloat {
                warn: 80.0,
                fail: 70.0,
            }),
        };
        let assessment = assess_coverage(Some(99.0), Some(&branch_only), &context);
        assert!(matches!(
            assessment.branch,
            ConfiguredMetricEvaluation::Missing
        ));
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
