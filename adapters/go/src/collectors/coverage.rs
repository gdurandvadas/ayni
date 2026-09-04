use super::util::run_tool_for_context;
use ayni_adapters_common::collector::{CollectorError, CollectorResult, CoverageBackedTestResult};
use ayni_adapters_common::exec::{
    format_command, run_command_for_context_streaming_structured,
    run_command_for_context_structured,
};
use ayni_adapters_common::failure::{
    command_failure_from_execution_error, command_failure_from_output, coverage_metric_failure,
    setup_failure,
};
use ayni_adapters_common::paths::to_repo_relative_path;
use ayni_adapters_common::reports::prepare_report_path;
use ayni_core::{
    Budget, ConfiguredMetricEvaluation, CoverageBudget, CoverageOffender, CoveragePolicy,
    CoverageResult, Language, Level, Offenders, RunContext, SignalKind, SignalResult, SignalRow,
    evaluate_configured_metric,
};
use std::fs;

pub fn collect(context: &RunContext) -> CollectorResult {
    let profile_path = prepare_coverage_profile(context)?;
    let (test_program, test_args, test_engine) =
        coverage_test_command(context, &profile_path, false);
    let test_output = match run_tool_for_context(context, &test_program, &test_args) {
        Ok(output) => output,
        Err(error) => {
            remove_profile(&profile_path);
            return Err(error.into());
        }
    };
    let coverage = build_coverage_row(
        context,
        &profile_path,
        &test_program,
        &test_args,
        test_engine,
        &test_output,
    );
    remove_profile(&profile_path);
    Ok(coverage)
}

pub fn collect_with_test_lines<F>(context: &RunContext, on_line: F) -> CoverageBackedTestResult
where
    F: FnMut(&str),
{
    let profile_path = prepare_coverage_profile(context)?;
    let (test_program, test_args, test_engine) =
        coverage_test_command(context, &profile_path, true);
    let output = match run_command_for_context_streaming_structured(
        context,
        &test_program,
        &test_args,
        on_line,
    ) {
        Ok(output) => output,
        Err(error) => {
            remove_profile(&profile_path);
            return Err(error.into());
        }
    };
    let test = super::test::build_row_from_output(
        context,
        &test_program,
        &test_args,
        &output,
        format_command(&test_program, &test_args),
    );
    let coverage = build_coverage_row(
        context,
        &profile_path,
        &test_program,
        &test_args,
        test_engine,
        &output,
    );
    remove_profile(&profile_path);
    Ok((test, coverage))
}

fn prepare_coverage_profile(context: &RunContext) -> Result<std::path::PathBuf, CollectorError> {
    prepare_report_path(context, "go", "coverage.out").map_err(CollectorError::Adapter)
}

fn remove_profile(profile_path: &std::path::Path) {
    let _ = fs::remove_file(profile_path);
}

fn build_coverage_row(
    context: &RunContext,
    profile_path: &std::path::Path,
    test_program: &str,
    test_args: &[String],
    test_engine: String,
    test_output: &std::process::Output,
) -> SignalRow {
    let (status, raw_line_percent, command_failure) = if test_output.status.success() {
        let cover_args = vec![
            String::from("tool"),
            String::from("cover"),
            String::from("-func"),
            profile_path.to_string_lossy().into_owned(),
        ];
        let cover_output = match run_command_for_context_structured(context, "go", &cover_args) {
            Ok(output) => output,
            Err(error) => {
                return coverage_error_row(
                    context,
                    format!("{test_engine} + go tool cover"),
                    command_failure_from_execution_error(SignalKind::Coverage, &error),
                );
            }
        };
        if cover_output.status.success() {
            match parse_total_percent(&String::from_utf8_lossy(&cover_output.stdout)) {
                Ok(percent) => (String::from("ok"), Some(percent), None),
                Err(message) => (
                    String::from("error"),
                    None,
                    Some(setup_failure(
                        context,
                        format!("{test_engine} + go tool cover"),
                        message,
                    )),
                ),
            }
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
        (
            String::from("error"),
            None,
            Some(command_failure_from_output(
                context,
                SignalKind::Coverage,
                test_program,
                test_args,
                test_output,
            )),
        )
    };

    let line_percent = finite_percent(raw_line_percent);
    let coverage_config = context.policy.go.coverage.as_ref();
    let coverage_budget = CoverageBudget {
        line_percent_warn: coverage_config.and_then(|config| config.line_percent.map(|v| v.warn)),
        line_percent_fail: coverage_config.and_then(|config| config.line_percent.map(|v| v.fail)),
        branch_percent_warn: coverage_config
            .and_then(|config| config.branch_percent.map(|v| v.warn)),
        branch_percent_fail: coverage_config
            .and_then(|config| config.branch_percent.map(|v| v.fail)),
    };

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
    let pass = status == "ok"
        && command_failure.is_none()
        && metric_failure.is_none()
        && !assessment.has_fail;

    SignalRow {
        kind: SignalKind::Coverage,
        language: Language::Go,
        scope: context.scope.clone(),
        pass,
        result: SignalResult::Coverage(CoverageResult {
            percent: line_percent,
            line_percent,
            branch_percent: None,
            engine: format!("{test_engine} + go tool cover"),
            status,
            failure: command_failure.or(metric_failure),
        }),
        budget: Budget::Coverage(coverage_budget),
        offenders: Offenders::Coverage(assessment.offenders),
    }
}

fn coverage_error_row(
    context: &RunContext,
    engine: String,
    failure: ayni_core::CommandFailure,
) -> SignalRow {
    SignalRow {
        kind: SignalKind::Coverage,
        language: Language::Go,
        scope: context.scope.clone(),
        pass: false,
        result: SignalResult::Coverage(CoverageResult {
            percent: None,
            line_percent: None,
            branch_percent: None,
            engine,
            status: String::from("error"),
            failure: Some(failure),
        }),
        budget: Budget::Coverage(CoverageBudget::default()),
        offenders: Offenders::Coverage(Vec::new()),
    }
}

fn coverage_test_command(
    context: &RunContext,
    profile_path: &std::path::Path,
    include_test_evidence: bool,
) -> (String, Vec<String>, String) {
    let profile_arg = format!("-coverprofile={}", profile_path.display());
    if let Some(override_cmd) = context
        .policy
        .tool_override_for(Language::Go, SignalKind::Coverage)
    {
        let mut args = if override_cmd.args.is_empty() {
            vec![String::from("test"), String::from("./...")]
        } else {
            override_cmd.args.clone()
        };
        ensure_go_coverage_args(&mut args, &profile_arg, include_test_evidence);
        let engine = format_command(&override_cmd.command, &args);
        return (override_cmd.command.clone(), args, engine);
    }
    let mut args = vec![String::from("test"), String::from("./...")];
    ensure_go_coverage_args(&mut args, &profile_arg, include_test_evidence);
    let engine = format_command("go", &args);
    (String::from("go"), args, engine)
}

fn ensure_go_coverage_args(args: &mut Vec<String>, profile_arg: &str, include_test_evidence: bool) {
    let mut normalized = Vec::with_capacity(args.len() + 2);
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        if arg == "-coverprofile" {
            index += usize::from(index + 1 < args.len());
        } else if !arg.starts_with("-coverprofile=") {
            normalized.push(arg.clone());
        }
        index += 1;
    }
    let insertion = normalized
        .iter()
        .position(|arg| arg == "-args")
        .unwrap_or(normalized.len());
    normalized.insert(insertion, profile_arg.to_string());
    if include_test_evidence
        && !normalized[..insertion]
            .iter()
            .any(|arg| arg == "-json" || arg.starts_with("-json="))
    {
        normalized.insert(insertion, String::from("-json"));
    }
    *args = normalized;
}

fn parse_total_percent(text: &str) -> Result<f64, String> {
    let Some(line) = text
        .lines()
        .rev()
        .find(|line| line.trim().starts_with("total:"))
    else {
        return Err(String::from(
            "go tool cover did not emit a total coverage percentage",
        ));
    };
    let Some(token) = line
        .split_whitespace()
        .last()
        .map(|value| value.trim_end_matches('%'))
    else {
        return Err(String::from(
            "go tool cover emitted a malformed total coverage percentage",
        ));
    };
    let percent = token.parse::<f64>().map_err(|_| {
        format!("go tool cover emitted an unparseable total coverage percentage: {token}")
    })?;
    if !percent.is_finite() {
        return Err(String::from(
            "go tool cover emitted a non-finite total coverage percentage",
        ));
    }
    Ok(percent)
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
            cancellation: Default::default(),
            debug: false,
        }
    }

    #[test]
    fn parses_total_percent_line() {
        let output = "pkg/a.go:10:\tfoo\t66.7%\ntotal:\t(statements)\t83.3%\n";
        assert_eq!(parse_total_percent(output), Ok(83.3));
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
        assert!(parse_total_percent("total:\t(statements)\tunknown%\n").is_err());
        assert!(parse_total_percent("no total\n").is_err());

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
    fn coverage_command_emits_json_and_cover_profile() {
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
        let (_, args, _) = coverage_test_command(
            &context,
            PathBuf::from(".ayni-go-cover.out").as_path(),
            true,
        );
        assert!(args.iter().any(|arg| arg == "-json"));
        assert!(
            args.iter()
                .any(|arg| arg == "-coverprofile=.ayni-go-cover.out")
        );
    }

    #[test]
    fn coverage_override_retains_explicit_args_and_adds_required_evidence() {
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
        let (program, args, engine) = coverage_test_command(
            &context,
            PathBuf::from(".ayni-go-cover.out").as_path(),
            true,
        );
        assert_eq!(program, "go");
        assert!(args.iter().any(|arg| arg == "-json"));
        assert!(
            args.iter()
                .any(|arg| arg == "-coverprofile=.ayni-go-cover.out")
        );
        assert!(engine.starts_with("go test ./..."));
    }

    #[test]
    fn coverage_override_replaces_custom_profile_destinations() {
        let context = context_with_policy(
            r#"
[languages]
enabled = ["go"]
[go.tooling.coverage]
command = "go"
args = ["test", "./...", "-coverprofile", "custom.out", "-coverprofile=other.out", "-args", "custom-test-arg"]
"#,
        );
        let (_, args, _) = coverage_test_command(
            &context,
            PathBuf::from(".ayni-go-cover.out").as_path(),
            true,
        );
        assert_eq!(
            args.iter()
                .filter(|arg| arg.starts_with("-coverprofile="))
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["-coverprofile=.ayni-go-cover.out"]
        );
        assert!(!args.iter().any(|arg| arg == "custom.out"));
        assert!(!args.iter().any(|arg| arg == "-coverprofile=other.out"));
        let test_args = args.iter().position(|arg| arg == "-args").expect("-args");
        assert!(args.iter().position(|arg| arg == "-json").unwrap() < test_args);
        assert!(
            args.iter()
                .position(|arg| arg.starts_with("-coverprofile="))
                .unwrap()
                < test_args
        );
    }
}
