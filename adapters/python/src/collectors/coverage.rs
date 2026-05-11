use super::util::{
    command_failure_from_output, command_for_override_or_default, format_command,
    prepare_report_path, run_command_for_context, to_repo_relative_path,
};
use ayni_core::{
    Budget, CoverageOffender, CoveragePolicy, CoverageResult, Language, Level, Offenders,
    RunContext, Scope, SignalKind, SignalResult, SignalRow,
};
use serde::Deserialize;
use serde_json::json;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

#[derive(Debug, Deserialize)]
struct CoverageJson {
    totals: Option<CoverageSummary>,
    files: Option<BTreeMap<String, CoverageFile>>,
}

#[derive(Debug, Deserialize)]
struct CoverageFile {
    summary: CoverageSummary,
}

#[derive(Debug, Deserialize)]
struct CoverageSummary {
    percent_covered: Option<f64>,
    #[serde(rename = "percent_covered_display")]
    percent_display: Option<String>,
}

pub fn collect(context: &RunContext) -> Result<SignalRow, String> {
    let report_path = prepare_report_path(context, "coverage.json")?;
    let cov_arg = format!("--cov-report=json:{}", report_path.display());
    let default_args = ["--cov=.", cov_arg.as_str()];
    let (program, args) =
        command_for_override_or_default(context, SignalKind::Coverage, "pytest", &default_args);
    let engine = format_command(&program, &args);
    let output = run_command_for_context(context, &program, &args)?;
    let status = if output.status.success() {
        "ok"
    } else {
        "error"
    };
    let failure = if output.status.success() {
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
            }),
            files: Some(BTreeMap::new()),
        },
        Err(_) if !output.status.success() => {
            return Ok(error_row(
                context,
                engine,
                failure.expect("coverage failure details"),
            ));
        }
        Err(error) => return Err(error),
    };
    let percent = report.totals.as_ref().and_then(percent_from_summary);
    let coverage_config = context.policy.python.coverage.as_ref();
    let coverage_budget = coverage_config
        .map(|config| {
            json!({
                "line_percent_warn": config.line_percent.map(|v| v.warn),
                "line_percent_fail": config.line_percent.map(|v| v.fail),
            })
        })
        .unwrap_or_else(|| json!({}));
    let offenders = build_offenders(report.files.as_ref(), coverage_config, context);
    let pass = status == "ok"
        && !offenders
            .iter()
            .any(|offender| offender.level == Level::Fail);

    Ok(SignalRow {
        kind: SignalKind::Coverage,
        language: Language::Python,
        scope: Scope {
            workspace_root: context.scope.workspace_root.clone(),
            path: context.scope.path.clone(),
            package: context.scope.package.clone(),
            file: context.scope.file.clone(),
        },
        pass,
        result: SignalResult::Coverage(CoverageResult {
            percent,
            line_percent: percent,
            branch_percent: None,
            engine,
            status: status.to_string(),
            failure,
        }),
        budget: Budget::Coverage(coverage_budget),
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
        language: Language::Python,
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

fn percent_from_summary(summary: &CoverageSummary) -> Option<f64> {
    summary.percent_covered.or_else(|| {
        summary
            .percent_display
            .as_ref()
            .and_then(|value| value.parse::<f64>().ok())
    })
}

fn build_offenders(
    files: Option<&BTreeMap<String, CoverageFile>>,
    policy: Option<&CoveragePolicy>,
    context: &RunContext,
) -> Vec<CoverageOffender> {
    let Some(threshold) = policy.and_then(|p| p.line_percent) else {
        return Vec::new();
    };
    let Some(files) = files else {
        return Vec::new();
    };
    let mut offenders = Vec::new();
    for (file, metrics) in files {
        let Some(value) = percent_from_summary(&metrics.summary) else {
            continue;
        };
        if value >= threshold.warn {
            continue;
        }
        let level = if value < threshold.fail {
            Level::Fail
        } else {
            Level::Warn
        };
        offenders.push(CoverageOffender {
            file: to_repo_relative_path(&context.repo_root, &context.workdir.join(file)),
            line: None,
            value,
            level,
        });
    }
    offenders.sort_by(|left, right| left.file.cmp(&right.file));
    offenders
}
