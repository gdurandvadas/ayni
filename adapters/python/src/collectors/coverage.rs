use super::util::{
    command_failure_from_output, command_for_override_or_default, format_command,
    prepare_report_path, run_command_for_context_streaming_structured,
    run_command_for_context_structured, to_repo_relative_path,
};
use ayni_adapters_common::collector::{CollectorError, CollectorResult, CoverageBackedTestResult};
use ayni_adapters_common::failure::{coverage_metric_failure, setup_failure};
use ayni_core::{
    Budget, ConfiguredMetricEvaluation, CoverageBudget, CoverageOffender, CoveragePolicy,
    CoverageResult, Language, Level, Offenders, RunContext, SignalKind, SignalResult, SignalRow,
    evaluate_configured_metric,
};
use serde::Deserialize;
use std::fs;
use std::path::Path;

#[derive(Debug, Deserialize)]
struct CoverageJson {
    totals: Option<CoverageSummary>,
}

#[derive(Clone, Debug, Deserialize)]
struct CoverageSummary {
    percent_covered: Option<f64>,
    #[serde(rename = "percent_covered_display")]
    percent_display: Option<String>,
    covered_lines: Option<f64>,
    num_statements: Option<f64>,
    covered_branches: Option<f64>,
    num_branches: Option<f64>,
}

pub fn collect(context: &RunContext) -> CollectorResult {
    let coverage_path =
        prepare_report_path(context, "coverage.json").map_err(CollectorError::Adapter)?;
    let (program, args) = coverage_command(context, None, &coverage_path);
    let output = run_command_for_context_structured(context, &program, &args)?;
    Ok(build_coverage_row(
        context,
        &program,
        &args,
        &coverage_path,
        &output,
    ))
}

pub fn collect_with_test_lines<F>(context: &RunContext, on_line: F) -> CoverageBackedTestResult
where
    F: FnMut(&str),
{
    let test_path =
        prepare_report_path(context, "pytest-report.json").map_err(CollectorError::Adapter)?;
    let coverage_path =
        prepare_report_path(context, "coverage.json").map_err(CollectorError::Adapter)?;
    let (program, args) = coverage_command(context, Some(&test_path), &coverage_path);
    let output = run_command_for_context_streaming_structured(context, &program, &args, on_line)?;
    let test = super::test::build_row_from_output(context, &program, &args, &test_path, &output);
    let coverage = build_coverage_row(context, &program, &args, &coverage_path, &output);
    Ok((test, coverage))
}

fn coverage_command(
    context: &RunContext,
    test_path: Option<&Path>,
    coverage_path: &Path,
) -> (String, Vec<String>) {
    let coverage_arg = format!("--cov-report=json:{}", coverage_path.display());
    let mut default_args = default_coverage_args(&coverage_arg)
        .into_iter()
        .map(String::from)
        .collect::<Vec<_>>();
    if let Some(test_path) = test_path {
        default_args.splice(
            0..0,
            [
                String::from("--json-report"),
                format!("--json-report-file={}", test_path.display()),
            ],
        );
    }
    let default_refs = default_args.iter().map(String::as_str).collect::<Vec<_>>();
    command_for_override_or_default(context, SignalKind::Coverage, "pytest", &default_refs)
}

fn build_coverage_row(
    context: &RunContext,
    program: &str,
    args: &[String],
    report_path: &Path,
    output: &std::process::Output,
) -> SignalRow {
    let engine = format_command(program, args);
    let command_failure = (!output.status.success())
        .then(|| command_failure_from_output(context, SignalKind::Coverage, program, args, output));
    let (report, report_failure) = load_coverage_report(context, report_path, output, &engine);
    let ((raw_line_percent, raw_branch_percent), evidence_failure) =
        validated_coverage_percents(context, &report, &engine);
    let mut failure = command_failure.or(report_failure).or(evidence_failure);
    let mut status = if failure.is_none() { "ok" } else { "error" };
    let line_percent = finite_percent(raw_line_percent);
    let branch_percent = finite_percent(raw_branch_percent);
    if line_percent.is_none() && branch_percent.is_none() && failure.is_none() {
        status = "error";
        failure = Some(setup_failure(
            context,
            engine.clone(),
            "coverage JSON did not contain a finite line or branch measurement",
        ));
    }
    let percent = line_percent.or(branch_percent);
    let coverage_config = context.policy.python.coverage.as_ref();
    let coverage_budget = applied_coverage_budget(coverage_config);
    let assessment = assess_coverage(
        raw_line_percent,
        raw_branch_percent,
        coverage_config,
        context,
    );
    let metric_failure = coverage_metric_failure(
        context,
        engine.clone(),
        "line_percent",
        assessment.line,
    )
    .or_else(|| {
        coverage_metric_failure(context, engine.clone(), "branch_percent", assessment.branch)
    });
    let pass =
        status == "ok" && failure.is_none() && metric_failure.is_none() && !assessment.has_fail;

    SignalRow {
        kind: SignalKind::Coverage,
        language: Language::Python,
        scope: context.scope.clone(),
        pass,
        result: SignalResult::Coverage(CoverageResult {
            percent,
            line_percent,
            branch_percent,
            engine,
            status: status.to_string(),
            failure: failure.or(metric_failure),
        }),
        budget: Budget::Coverage(coverage_budget),
        offenders: Offenders::Coverage(assessment.offenders),
    }
}

fn load_coverage_report(
    context: &RunContext,
    report_path: &Path,
    output: &std::process::Output,
    engine: &str,
) -> (CoverageJson, Option<ayni_core::CommandFailure>) {
    match read_report(report_path) {
        Ok(report) => (report, None),
        Err(_) if is_no_tests_collected(output) => (empty_run_coverage_report(), None),
        Err(error) => {
            let malformed = report_path.exists();
            let report = CoverageJson {
                totals: Some(CoverageSummary {
                    percent_covered: malformed.then_some(f64::NAN),
                    percent_display: None,
                    covered_lines: malformed.then_some(f64::NAN),
                    num_statements: malformed.then_some(1.0),
                    covered_branches: malformed.then_some(f64::NAN),
                    num_branches: malformed.then_some(1.0),
                }),
            };
            let failure = ayni_core::CommandFailure {
                category: String::from("repo_setup_issue"),
                classification: String::from("unparseable_coverage_report"),
                command: engine.to_string(),
                cwd: context.execution.exec_cwd.display().to_string(),
                exit_code: None,
                message: error,
            };
            (report, Some(failure))
        }
    }
}

fn empty_run_coverage_report() -> CoverageJson {
    CoverageJson {
        totals: Some(CoverageSummary {
            percent_covered: Some(0.0),
            percent_display: Some(String::from("0")),
            covered_lines: Some(0.0),
            num_statements: Some(1.0),
            covered_branches: None,
            num_branches: None,
        }),
    }
}

fn validated_coverage_percents(
    context: &RunContext,
    report: &CoverageJson,
    engine: &str,
) -> (
    (Option<f64>, Option<f64>),
    Option<ayni_core::CommandFailure>,
) {
    let Some(summary) = report.totals.as_ref() else {
        return ((None, None), None);
    };
    match validate_coverage_summary(summary) {
        Ok(()) => (coverage_percents(summary), None),
        Err(message) => (
            (None, None),
            Some(setup_failure(context, engine.to_string(), message)),
        ),
    }
}

fn default_coverage_args(report_argument: &str) -> [&str; 3] {
    ["--cov=.", "--cov-branch", report_argument]
}

fn applied_coverage_budget(config: Option<&CoveragePolicy>) -> CoverageBudget {
    CoverageBudget {
        line_percent_warn: config.and_then(|value| value.line_percent.map(|v| v.warn)),
        line_percent_fail: config.and_then(|value| value.line_percent.map(|v| v.fail)),
        branch_percent_warn: config.and_then(|value| value.branch_percent.map(|v| v.warn)),
        branch_percent_fail: config.and_then(|value| value.branch_percent.map(|v| v.fail)),
    }
}

fn is_no_tests_collected(output: &std::process::Output) -> bool {
    if output.status.code() == Some(5) {
        return true;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    stdout.contains("no tests ran") || stderr.contains("no tests ran")
}

fn read_report(path: &Path) -> Result<CoverageJson, String> {
    let content = fs::read_to_string(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    serde_json::from_str(&content)
        .map_err(|error| format!("failed to parse {}: {error}", path.display()))
}

fn validate_coverage_summary(summary: &CoverageSummary) -> Result<(), String> {
    validate_percent("percent_covered", summary.percent_covered)?;
    if let Some(display) = &summary.percent_display {
        let value = display
            .parse::<f64>()
            .map_err(|_| String::from("coverage JSON percent_covered_display was not numeric"))?;
        validate_percent("percent_covered_display", Some(value))?;
    }
    validate_count_pair(
        "covered_lines",
        summary.covered_lines,
        "num_statements",
        summary.num_statements,
    )?;
    validate_count_pair(
        "covered_branches",
        summary.covered_branches,
        "num_branches",
        summary.num_branches,
    )
}

fn validate_percent(name: &str, value: Option<f64>) -> Result<(), String> {
    if value.is_some_and(|value| !value.is_finite() || !(0.0..=100.0).contains(&value)) {
        return Err(format!(
            "coverage JSON {name} must be finite and between 0 and 100"
        ));
    }
    Ok(())
}

fn validate_count_pair(
    covered_name: &str,
    covered: Option<f64>,
    total_name: &str,
    total: Option<f64>,
) -> Result<(), String> {
    for (name, value) in [(covered_name, covered), (total_name, total)] {
        if value.is_some_and(|value| {
            !value.is_finite() || value < 0.0 || value.fract() != 0.0 || value >= u64::MAX as f64
        }) {
            return Err(format!(
                "coverage JSON {name} must be a finite non-negative integer within range"
            ));
        }
    }
    if covered.is_some() != total.is_some() {
        return Err(format!(
            "coverage JSON {covered_name} and {total_name} must be present together"
        ));
    }
    if let (Some(covered), Some(total)) = (covered, total)
        && covered > total
    {
        return Err(format!(
            "coverage JSON {covered_name} cannot exceed {total_name}"
        ));
    }
    Ok(())
}

fn coverage_percents(summary: &CoverageSummary) -> (Option<f64>, Option<f64>) {
    (
        ratio_percent(summary.covered_lines, summary.num_statements)
            .or_else(|| percent_from_summary(summary)),
        ratio_percent(summary.covered_branches, summary.num_branches),
    )
}

fn percent_from_summary(summary: &CoverageSummary) -> Option<f64> {
    summary.percent_covered.or_else(|| {
        summary
            .percent_display
            .as_ref()
            .map(|value| value.parse::<f64>().unwrap_or(f64::NAN))
    })
}

fn ratio_percent(covered: Option<f64>, total: Option<f64>) -> Option<f64> {
    match (covered, total) {
        (Some(covered), Some(total)) if total > 0.0 => Some(covered * 100.0 / total),
        (Some(_), Some(_)) => Some(f64::NAN),
        _ => None,
    }
}

struct CoverageAssessment {
    line: ConfiguredMetricEvaluation,
    branch: ConfiguredMetricEvaluation,
    offenders: Vec<CoverageOffender>,
    has_fail: bool,
}

fn assess_coverage(
    line_percent: Option<f64>,
    branch_percent: Option<f64>,
    policy: Option<&CoveragePolicy>,
    context: &RunContext,
) -> CoverageAssessment {
    let line = evaluate_configured_metric(line_percent, policy.and_then(|p| p.line_percent));
    let branch = evaluate_configured_metric(branch_percent, policy.and_then(|p| p.branch_percent));
    let mut offenders = Vec::new();
    for evaluation in [line, branch] {
        if let ConfiguredMetricEvaluation::Present {
            value,
            level: Some(level),
        } = evaluation
        {
            offenders.push(CoverageOffender {
                file: to_repo_relative_path(&context.repo_root, &context.workdir),
                line: None,
                value,
                level,
            });
        }
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
    use super::{
        assess_coverage, coverage_command, coverage_percents, default_coverage_args,
        validate_coverage_summary,
    };
    use ayni_core::{
        AyniPolicy, ConfiguredMetricEvaluation, CoveragePolicy, ExecutionResolution, Level,
        RunContext, Scope, SignalResult, ThresholdFloat,
    };
    use std::fs;
    use std::path::PathBuf;
    use std::process::Command;
    use tempfile::TempDir;

    fn context() -> RunContext {
        RunContext {
            repo_root: PathBuf::from("."),
            target_root: PathBuf::from("."),
            workdir: PathBuf::from("."),
            policy: AyniPolicy::default(),
            scope: Scope::default(),
            execution: ExecutionResolution::direct("pytest", PathBuf::from("."), "test", 100),
            cancellation: Default::default(),
            debug: false,
        }
    }

    fn policy() -> CoveragePolicy {
        CoveragePolicy {
            line_percent: Some(ThresholdFloat {
                warn: 80.0,
                fail: 70.0,
            }),
            branch_percent: Some(ThresholdFloat {
                warn: 60.0,
                fail: 50.0,
            }),
        }
    }

    #[test]
    fn default_command_collects_branches() {
        assert!(default_coverage_args("--cov-report=json:coverage.json").contains(&"--cov-branch"));
    }

    #[test]
    fn empty_coverage_override_adds_both_combined_reporters() {
        let mut context = context();
        context.policy = toml::from_str(
            r#"
[checks]
test = true
coverage = true
[languages]
enabled = ["python"]
[python.tooling]
coverage_satisfies_test = true
[python.tooling.coverage]
command = "pytest"
"#,
        )
        .expect("policy");
        let test_path = PathBuf::from("pytest-report.json");
        let coverage_path = PathBuf::from("coverage.json");
        let (program, args) = coverage_command(&context, Some(&test_path), &coverage_path);
        assert_eq!(program, "pytest");
        assert!(args.iter().any(|arg| arg == "--json-report"));
        assert!(
            args.iter()
                .any(|arg| arg == "--json-report-file=pytest-report.json")
        );
        assert!(
            args.iter()
                .any(|arg| arg == "--cov-report=json:coverage.json")
        );
    }

    #[test]
    fn explicit_coverage_override_args_are_preserved_as_attestation() {
        let mut context = context();
        context.policy = toml::from_str(
            r#"
[checks]
test = true
coverage = true
[languages]
enabled = ["python"]
[python.tooling]
coverage_satisfies_test = true
[python.tooling.coverage]
command = "custom-pytest"
args = ["--reports-are-configured-elsewhere"]
"#,
        )
        .expect("policy");
        let (program, args) = coverage_command(
            &context,
            Some(&PathBuf::from("pytest-report.json")),
            &PathBuf::from("coverage.json"),
        );
        assert_eq!(program, "custom-pytest");
        assert_eq!(args, ["--reports-are-configured-elsewhere"]);
    }

    #[test]
    fn independently_enforces_boundaries_and_measured_zero() {
        let context = context();
        let policy = policy();
        assert!(
            assess_coverage(Some(80.0), Some(60.0), Some(&policy), &context)
                .offenders
                .is_empty()
        );
        let equal_fail = assess_coverage(Some(70.0), Some(50.0), Some(&policy), &context);
        assert!(
            equal_fail
                .offenders
                .iter()
                .all(|offender| offender.level == Level::Warn)
        );
        assert!(assess_coverage(Some(69.0), Some(49.0), Some(&policy), &context).has_fail);
        let zero = assess_coverage(Some(0.0), Some(0.0), Some(&policy), &context);
        assert!(zero.has_fail);
        assert!(zero.offenders.iter().all(|offender| offender.value == 0.0));
    }

    #[test]
    fn override_without_branch_evidence_fails_closed() {
        let assessment = assess_coverage(Some(90.0), None, Some(&policy()), &context());
        assert!(matches!(
            assessment.branch,
            ConfiguredMetricEvaluation::Missing
        ));
        let malformed = assess_coverage(Some(f64::NAN), Some(60.0), Some(&policy()), &context());
        assert!(matches!(
            malformed.line,
            ConfiguredMetricEvaluation::Unparseable
        ));
    }

    #[test]
    fn rejects_invalid_coverage_counts_before_percentage_construction() {
        let valid = super::CoverageSummary {
            percent_covered: Some(75.0),
            percent_display: None,
            covered_lines: Some(3.0),
            num_statements: Some(4.0),
            covered_branches: Some(1.0),
            num_branches: Some(4.0),
        };
        assert!(validate_coverage_summary(&valid).is_ok());

        for invalid in [
            super::CoverageSummary {
                covered_lines: Some(-1.0),
                ..valid.clone()
            },
            super::CoverageSummary {
                num_statements: Some(f64::NAN),
                ..valid.clone()
            },
            super::CoverageSummary {
                covered_branches: Some(5.0),
                num_branches: Some(4.0),
                ..valid.clone()
            },
            super::CoverageSummary {
                covered_lines: Some((u64::MAX as f64) * 2.0),
                ..valid.clone()
            },
            super::CoverageSummary {
                num_statements: None,
                ..valid.clone()
            },
        ] {
            assert!(validate_coverage_summary(&invalid).is_err());
        }
    }

    #[test]
    fn invalid_counts_produce_a_typed_failed_row_without_invalid_percentages() {
        let temp = TempDir::new().expect("temporary report directory");
        let report_path = temp.path().join("coverage.json");
        fs::write(
            &report_path,
            r#"{"totals":{"covered_lines":-1,"num_statements":1}}"#,
        )
        .expect("report fixture");
        let output = Command::new("sh")
            .args(["-c", "exit 0"])
            .output()
            .expect("successful command output");
        let row = super::build_coverage_row(&context(), "pytest", &[], &report_path, &output);
        assert!(!row.pass);
        let SignalResult::Coverage(result) = row.result else {
            panic!("coverage result")
        };
        assert_eq!(
            (result.percent, result.line_percent, result.branch_percent),
            (None, None, None)
        );
        assert!(
            result
                .failure
                .expect("invalid evidence failure")
                .message
                .contains("non-negative integer")
        );
    }

    #[test]
    fn derives_line_and_branch_percentages_from_documented_totals() {
        let summary = super::CoverageSummary {
            percent_covered: Some(99.0),
            percent_display: None,
            covered_lines: Some(3.0),
            num_statements: Some(4.0),
            covered_branches: Some(1.0),
            num_branches: Some(4.0),
        };
        assert_eq!(coverage_percents(&summary), (Some(75.0), Some(25.0)));
    }
}
