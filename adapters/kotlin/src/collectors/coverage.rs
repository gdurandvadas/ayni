use super::util::{
    attr_u64, command_failure_from_output, find_report, format_command, gradle_command,
    run_command_for_context, setup_failure, to_repo_relative_path,
};
use ayni_core::{
    Budget, CoverageOffender, CoveragePolicy, CoverageResult, Language, Level, Offenders,
    RunContext, Scope, SignalKind, SignalResult, SignalRow,
};
use regex::Regex;
use serde_json::json;
use std::fs;
use std::path::Path;

pub fn collect(context: &RunContext) -> Result<SignalRow, String> {
    let (program, args) = gradle_command(context, SignalKind::Coverage, "koverXmlReport");
    let engine = format_command(&program, &args);
    let output = run_command_for_context(context, &program, &args)?;
    if !output.status.success() {
        return Ok(error_row(
            context,
            engine,
            command_failure_from_output(context, SignalKind::Coverage, &program, &args, &output),
        ));
    }
    let Some(report_path) = find_report(&context.workdir, &["build", "reports", "kover"], "xml")
    else {
        return Ok(error_row(
            context,
            engine,
            setup_failure(
                context,
                format_command(&program, &args),
                "koverXmlReport did not produce a Kover XML report under build/reports/kover",
            ),
        ));
    };
    let report = parse_jacoco_xml(&report_path)?;
    let coverage_config = context.policy.kotlin.coverage.as_ref();
    let budget = coverage_config
        .map(|config| {
            json!({
                "line_percent_warn": config.line_percent.map(|v| v.warn),
                "line_percent_fail": config.line_percent.map(|v| v.fail),
            })
        })
        .unwrap_or_else(|| json!({}));
    let offenders = build_offenders(report.line_percent, coverage_config, context);
    let pass = !offenders
        .iter()
        .any(|offender| offender.level == Level::Fail);

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
            percent: report.line_percent,
            line_percent: report.line_percent,
            branch_percent: report.branch_percent,
            engine,
            status: String::from("ok"),
            failure: None,
        }),
        budget: Budget::Coverage(budget),
        offenders: Offenders::Coverage(offenders),
        delta_vs_previous: None,
        delta_vs_baseline: None,
    })
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
        delta_vs_previous: None,
        delta_vs_baseline: None,
    }
}

#[derive(Debug, Default)]
struct CoverageReport {
    line_percent: Option<f64>,
    branch_percent: Option<f64>,
}

fn parse_jacoco_xml(path: &Path) -> Result<CoverageReport, String> {
    let content = fs::read_to_string(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    parse_jacoco_content(&content)
}

fn parse_jacoco_content(content: &str) -> Result<CoverageReport, String> {
    let counter_re = Regex::new(r#"<counter\b([^>]*)/>"#)
        .map_err(|error| format!("failed to compile counter regex: {error}"))?;
    let mut report = CoverageReport::default();
    for caps in counter_re.captures_iter(content) {
        let attrs = caps.get(1).map(|value| value.as_str()).unwrap_or("");
        let missed = attr_u64(attrs, "missed").unwrap_or(0);
        let covered = attr_u64(attrs, "covered").unwrap_or(0);
        let percent = percent(covered, missed);
        if attrs.contains(r#"type="LINE""#) {
            report.line_percent = percent;
        } else if attrs.contains(r#"type="BRANCH""#) {
            report.branch_percent = percent;
        }
    }
    Ok(report)
}

fn percent(covered: u64, missed: u64) -> Option<f64> {
    let total = covered + missed;
    (total > 0).then_some((covered as f64 / total as f64) * 100.0)
}

fn build_offenders(
    headline: Option<f64>,
    policy: Option<&CoveragePolicy>,
    context: &RunContext,
) -> Vec<CoverageOffender> {
    let Some(value) = headline else {
        return Vec::new();
    };
    let Some(threshold) = policy.and_then(|policy| policy.line_percent) else {
        return Vec::new();
    };
    if value >= threshold.warn {
        return Vec::new();
    }
    vec![CoverageOffender {
        file: to_repo_relative_path(&context.repo_root, &context.workdir),
        line: None,
        value,
        level: if value < threshold.fail {
            Level::Fail
        } else {
            Level::Warn
        },
    }]
}

#[cfg(test)]
mod tests {
    use super::parse_jacoco_content;

    #[test]
    fn parses_jacoco_counters() {
        let report = parse_jacoco_content(
            r#"<report><counter type="LINE" missed="2" covered="8"/><counter type="BRANCH" missed="1" covered="3"/></report>"#,
        )
        .expect("coverage");

        assert_eq!(report.line_percent, Some(80.0));
        assert_eq!(report.branch_percent, Some(75.0));
    }
}
