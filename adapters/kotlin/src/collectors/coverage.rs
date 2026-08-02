use super::util::{find_reports, gradle_command, resolve_gradle_task};
use ayni_adapters_common::collector::{CollectorError, CollectorResult};
use ayni_adapters_common::exec::{format_command, run_command_for_context_structured};
use ayni_adapters_common::failure::{
    command_failure_from_output, coverage_metric_failure, setup_failure,
};
use ayni_adapters_common::paths::to_repo_relative_path;
use ayni_adapters_common::xml::{attr_string, attr_u64};
use ayni_core::{
    Budget, ConfiguredMetricEvaluation, CoverageOffender, CoveragePolicy, CoverageResult, Language,
    Level, Offenders, RunContext, Scope, SignalKind, SignalResult, SignalRow,
    evaluate_configured_metric,
};
use regex::Regex;
use serde_json::json;
use std::fs;
use std::path::Path;

pub fn collect(context: &RunContext) -> CollectorResult {
    let task = resolve_coverage_task(context)?;
    let (program, args) = gradle_command(context, SignalKind::Coverage, &task);
    let engine = format_command(&program, &args);
    let output = run_command_for_context_structured(context, &program, &args)?;
    if !output.status.success() {
        return Ok(error_row(
            context,
            engine,
            command_failure_from_output(context, SignalKind::Coverage, &program, &args, &output),
        ));
    }
    let report_paths = coverage_report_paths(context, &task);
    if report_paths.is_empty() {
        return Ok(error_row(
            context,
            engine,
            setup_failure(
                context,
                format_command(&program, &args),
                missing_coverage_report_message(&task),
            ),
        ));
    }
    let mut totals = CoverageCounters::default();
    for path in &report_paths {
        totals.merge(parse_jacoco_xml(path).map_err(CollectorError::Adapter)?);
    }
    let report = totals.finish();
    let coverage_config = context.policy.kotlin.coverage.as_ref();
    let budget = coverage_config
        .map(|config| {
            json!({
                "line_percent_warn": config.line_percent.map(|v| v.warn),
                "line_percent_fail": config.line_percent.map(|v| v.fail),
                "branch_percent_warn": config.branch_percent.map(|v| v.warn),
                "branch_percent_fail": config.branch_percent.map(|v| v.fail),
            })
        })
        .unwrap_or_else(|| json!({}));
    let assessment = assess_coverage(
        report.raw_line_percent,
        report.raw_branch_percent,
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
    let pass = metric_failure.is_none() && !assessment.has_fail;

    Ok(SignalRow {
        kind: SignalKind::Coverage,
        language: Language::Kotlin,
        scope: Scope {
            workspace_root: context.scope.workspace_root.clone(),
            path: context.scope.path.clone(),
            package: context.scope.package.clone(),
            file: context.scope.file.clone(),
        },
        pass,
        result: SignalResult::Coverage(CoverageResult {
            percent: report.line_percent.or(report.branch_percent),
            line_percent: report.line_percent,
            branch_percent: report.branch_percent,
            engine,
            status: String::from("ok"),
            failure: metric_failure,
        }),
        budget: Budget::Coverage(budget),
        offenders: Offenders::Coverage(assessment.offenders),
    })
}

fn resolve_coverage_task(
    context: &RunContext,
) -> Result<String, Box<ayni_adapters_common::exec::ExecutionError>> {
    if context
        .policy
        .tool_override_for(Language::Kotlin, SignalKind::Coverage)
        .is_some()
    {
        return Ok(String::from("koverXmlReport"));
    }

    Ok(
        resolve_gradle_task(context, &["koverXmlReport", "jacocoTestReport"])?
            .unwrap_or_else(|| String::from("koverXmlReport")),
    )
}

fn coverage_report_paths(context: &RunContext, task: &str) -> Vec<std::path::PathBuf> {
    match task {
        "jacocoTestReport" => {
            find_reports(&context.workdir, &["build", "reports", "jacoco"], "xml")
        }
        _ => find_reports(&context.workdir, &["build", "reports", "kover"], "xml"),
    }
}

fn missing_coverage_report_message(task: &str) -> &'static str {
    match task {
        "jacocoTestReport" => {
            "jacocoTestReport did not produce a JaCoCo XML report under build/reports/jacoco"
        }
        _ => "koverXmlReport did not produce a Kover XML report under build/reports/kover",
    }
}

fn error_row(
    context: &RunContext,
    engine: String,
    failure: ayni_core::CommandFailure,
) -> SignalRow {
    SignalRow {
        kind: SignalKind::Coverage,
        language: Language::Kotlin,
        scope: Scope {
            workspace_root: context.scope.workspace_root.clone(),
            path: context.scope.path.clone(),
            package: context.scope.package.clone(),
            file: context.scope.file.clone(),
        },
        pass: false,
        result: SignalResult::Coverage(CoverageResult {
            percent: None,
            line_percent: None,
            branch_percent: None,
            engine,
            status: String::from("error"),
            failure: Some(failure),
        }),
        budget: Budget::Coverage(json!({})),
        offenders: Offenders::Coverage(Vec::new()),
    }
}

#[derive(Debug, Default)]
struct CoverageReport {
    line: MetricCounter,
    branch: MetricCounter,
}

#[derive(Debug, Default)]
struct CoverageCounters {
    line: MetricCounter,
    branch: MetricCounter,
}

impl CoverageCounters {
    fn merge(&mut self, report: CoverageReport) {
        self.line.merge(report.line);
        self.branch.merge(report.branch);
    }

    fn finish(&self) -> CoverageTotals {
        CoverageTotals {
            raw_line_percent: self.line.percent(),
            raw_branch_percent: self.branch.percent(),
            line_percent: finite_percent(self.line.percent()),
            branch_percent: finite_percent(self.branch.percent()),
        }
    }
}

#[derive(Debug, Default)]
struct CoverageTotals {
    raw_line_percent: Option<f64>,
    raw_branch_percent: Option<f64>,
    line_percent: Option<f64>,
    branch_percent: Option<f64>,
}

#[derive(Debug, Default)]
enum MetricCounter {
    #[default]
    Absent,
    Values {
        covered: u64,
        missed: u64,
    },
    Invalid,
}

impl MetricCounter {
    fn merge(&mut self, other: Self) {
        match other {
            Self::Absent => {}
            Self::Invalid => *self = Self::Invalid,
            Self::Values { covered, missed } => match self {
                Self::Absent => *self = Self::Values { covered, missed },
                Self::Invalid => {}
                Self::Values {
                    covered: total_covered,
                    missed: total_missed,
                } => match (
                    total_covered.checked_add(covered),
                    total_missed.checked_add(missed),
                ) {
                    (Some(covered), Some(missed)) => {
                        *total_covered = covered;
                        *total_missed = missed;
                    }
                    _ => *self = Self::Invalid,
                },
            },
        }
    }

    fn percent(&self) -> Option<f64> {
        match self {
            Self::Values { covered, missed } => {
                Some(percent(*covered, *missed).unwrap_or(f64::NAN))
            }
            Self::Absent => None,
            Self::Invalid => Some(f64::NAN),
        }
    }
}

fn parse_jacoco_xml(path: &Path) -> Result<CoverageReport, String> {
    let content = fs::read_to_string(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    parse_jacoco_content(&content)
}

fn parse_jacoco_content(content: &str) -> Result<CoverageReport, String> {
    let counter_re = Regex::new(r#"<counter\b([^>]*)/>"#)
        .map_err(|error| format!("failed to compile counter regex: {error}"))?;
    let mut line = MetricCounter::Absent;
    let mut branch = MetricCounter::Absent;
    for caps in counter_re.captures_iter(content) {
        let attrs = caps.get(1).map(|value| value.as_str()).unwrap_or("");
        let counter = match (attr_u64(attrs, "covered"), attr_u64(attrs, "missed")) {
            (Some(covered), Some(missed)) => MetricCounter::Values { covered, missed },
            _ => MetricCounter::Invalid,
        };
        match attr_string(attrs, "type").as_deref() {
            Some("LINE") => line = counter,
            Some("BRANCH") => branch = counter,
            _ => {}
        }
    }
    Ok(CoverageReport { line, branch })
}

fn percent(covered: u64, missed: u64) -> Option<f64> {
    let total = covered.checked_add(missed)?;
    (total > 0).then_some((covered as f64 / total as f64) * 100.0)
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
    let line =
        evaluate_configured_metric(line_percent, policy.and_then(|policy| policy.line_percent));
    let branch = evaluate_configured_metric(
        branch_percent,
        policy.and_then(|policy| policy.branch_percent),
    );
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
    use super::{CoverageCounters, assess_coverage, parse_jacoco_content};
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
            execution: ExecutionResolution::direct("gradle", PathBuf::from("."), "test", 100),
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
    fn parses_jacoco_counters() {
        let report = parse_jacoco_content(
            r#"<report><counter type="LINE" missed="2" covered="8"/><counter type="BRANCH" missed="1" covered="3"/></report>"#,
        )
        .expect("coverage");

        let mut totals = CoverageCounters::default();
        totals.merge(report);
        let finished = totals.finish();
        assert_eq!(finished.line_percent, Some(80.0));
        assert_eq!(finished.branch_percent, Some(75.0));
    }

    #[test]
    fn aggregates_counters_across_reports() {
        let mut totals = CoverageCounters::default();
        totals.merge(
            parse_jacoco_content(
                r#"<report><counter type="LINE" missed="0" covered="10"/></report>"#,
            )
            .expect("first"),
        );
        totals.merge(
            parse_jacoco_content(
                r#"<report><counter type="LINE" missed="10" covered="30"/></report>"#,
            )
            .expect("second"),
        );
        let finished = totals.finish();
        assert_eq!(finished.line_percent, Some(80.0));
    }

    #[test]
    fn independently_enforces_line_and_branch_threshold_boundaries() {
        let context = context();
        let policy = policy();
        let equal_warn = assess_coverage(Some(80.0), Some(60.0), Some(&policy), &context);
        assert!(equal_warn.offenders.is_empty());

        let equal_fail = assess_coverage(Some(70.0), Some(50.0), Some(&policy), &context);
        assert_eq!(equal_fail.offenders.len(), 2);
        assert!(
            equal_fail
                .offenders
                .iter()
                .all(|offender| offender.level == Level::Warn)
        );
        assert!(!equal_fail.has_fail);

        let below_fail = assess_coverage(Some(69.0), Some(49.0), Some(&policy), &context);
        assert!(below_fail.has_fail);
        assert!(
            below_fail
                .offenders
                .iter()
                .all(|offender| offender.level == Level::Fail)
        );
    }

    #[test]
    fn preserves_measured_zero_as_numeric_evidence() {
        let context = context();
        let assessment = assess_coverage(Some(0.0), Some(0.0), Some(&policy()), &context);
        assert!(assessment.has_fail);
        assert!(
            assessment
                .offenders
                .iter()
                .all(|offender| offender.value == 0.0)
        );
    }

    #[test]
    fn rejects_missing_required_line_and_branch_counters() {
        let context = context();
        let policy = policy();
        let empty = parse_jacoco_content("<report/>").expect("report");
        let mut totals = CoverageCounters::default();
        totals.merge(empty);
        let finished = totals.finish();
        let assessment = assess_coverage(
            finished.raw_line_percent,
            finished.raw_branch_percent,
            Some(&policy),
            &context,
        );
        assert!(matches!(
            assessment.line,
            ConfiguredMetricEvaluation::Missing
        ));
        assert!(matches!(
            assessment.branch,
            ConfiguredMetricEvaluation::Missing
        ));
    }

    #[test]
    fn rejects_malformed_and_invalid_counter_arithmetic() {
        let context = context();
        let policy = policy();
        let malformed = parse_jacoco_content(
            r#"<report><counter type="LINE" missed="bad" covered="8"/><counter type="BRANCH" missed="1" covered="3"/></report>"#,
        )
        .expect("report");
        let mut totals = CoverageCounters::default();
        totals.merge(malformed);
        let finished = totals.finish();
        let assessment = assess_coverage(
            finished.raw_line_percent,
            finished.raw_branch_percent,
            Some(&policy),
            &context,
        );
        assert!(matches!(
            assessment.line,
            ConfiguredMetricEvaluation::Unparseable
        ));

        let overflow = parse_jacoco_content(
            r#"<report><counter type="LINE" missed="1" covered="18446744073709551615"/><counter type="BRANCH" missed="0" covered="1"/></report>"#,
        )
        .expect("report");
        let mut totals = CoverageCounters::default();
        totals.merge(overflow);
        let finished = totals.finish();
        let assessment = assess_coverage(
            finished.raw_line_percent,
            finished.raw_branch_percent,
            Some(&policy),
            &context,
        );
        assert!(matches!(
            assessment.line,
            ConfiguredMetricEvaluation::Unparseable
        ));
    }
}
