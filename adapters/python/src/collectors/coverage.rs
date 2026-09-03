use super::util::{
    command_failure_from_output, command_for_override_or_default, format_command,
    prepare_report_path, run_command_for_context_structured, to_repo_relative_path,
};
use ayni_adapters_common::collector::{CollectorError, CollectorResult};
use ayni_adapters_common::failure::coverage_metric_failure;
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

#[derive(Debug, Deserialize)]
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
    let report_path =
        prepare_report_path(context, "coverage.json").map_err(CollectorError::Adapter)?;
    let cov_arg = format!("--cov-report=json:{}", report_path.display());
    let default_args = default_coverage_args(&cov_arg);
    let (program, args) =
        command_for_override_or_default(context, SignalKind::Coverage, "pytest", &default_args);
    let engine = format_command(&program, &args);
    let output = run_command_for_context_structured(context, &program, &args)?;
    let mut status = if output.status.success() {
        "ok"
    } else {
        "error"
    };
    let mut failure = if output.status.success() {
        None
    } else {
        Some(command_failure_from_output(
            context,
            SignalKind::Coverage,
            &program,
            &args,
            &output,
        ))
    };

    let report = match read_report(&report_path) {
        Ok(report) => report,
        Err(_) if is_no_tests_collected(&output) => CoverageJson {
            totals: Some(CoverageSummary {
                percent_covered: Some(0.0),
                percent_display: Some(String::from("0")),
                covered_lines: Some(0.0),
                num_statements: Some(1.0),
                covered_branches: None,
                num_branches: None,
            }),
        },
        Err(_) if !output.status.success() => {
            return Ok(error_row(
                context,
                engine,
                failure.expect("coverage failure details"),
            ));
        }
        Err(error) => {
            status = "error";
            let malformed = report_path.exists();
            failure = Some(ayni_core::CommandFailure {
                category: String::from("repo_setup_issue"),
                classification: String::from("unparseable_coverage_report"),
                command: engine.clone(),
                cwd: context.execution.exec_cwd.display().to_string(),
                exit_code: None,
                message: error,
            });
            CoverageJson {
                totals: Some(CoverageSummary {
                    percent_covered: malformed.then_some(f64::NAN),
                    percent_display: None,
                    covered_lines: malformed.then_some(f64::NAN),
                    num_statements: malformed.then_some(1.0),
                    covered_branches: malformed.then_some(f64::NAN),
                    num_branches: malformed.then_some(1.0),
                }),
            }
        }
    };
    let (raw_line_percent, raw_branch_percent) = report
        .totals
        .as_ref()
        .map(coverage_percents)
        .unwrap_or((None, None));
    let line_percent = finite_percent(raw_line_percent);
    let branch_percent = finite_percent(raw_branch_percent);
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
    let pass = status == "ok" && metric_failure.is_none() && !assessment.has_fail;

    Ok(SignalRow {
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
    })
}

fn default_coverage_args(report_argument: &str) -> [&str; 3] {
    ["--cov=.", "--cov-branch", report_argument]
}

fn error_row(
    context: &RunContext,
    engine: String,
    failure: ayni_core::CommandFailure,
) -> SignalRow {
    SignalRow {
        kind: SignalKind::Coverage,
        language: Language::Python,
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
        budget: Budget::Coverage(applied_coverage_budget(
            context.policy.python.coverage.as_ref(),
        )),
        offenders: Offenders::Coverage(Vec::new()),
    }
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
    use super::{assess_coverage, coverage_percents, default_coverage_args};
    use ayni_core::{
        AyniPolicy, ConfiguredMetricEvaluation, CoveragePolicy, ExecutionResolution, Level,
        RunContext, Scope, ThresholdFloat,
    };
    use std::path::PathBuf;

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
