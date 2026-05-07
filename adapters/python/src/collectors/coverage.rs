use super::util::{
    command_for_override_or_default, ensure_ayni_dir, run_command, to_repo_relative_path,
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
    let artifact_dir = ensure_ayni_dir(context)?;
    let report_path = artifact_dir.join("coverage.json");
    let cov_arg = format!("--cov-report=json:{}", report_path.display());
    let default_args = ["--cov=.", cov_arg.as_str()];
    let (program, args) =
        command_for_override_or_default(context, SignalKind::Coverage, "pytest", &default_args);
    let engine = format_command(&program, &args);
    let output = run_command(&context.workdir, &program, &args)?;
    let status = if output.status.success() {
        "ok"
    } else {
        "error"
    };

    let report = read_report(&report_path)?;
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
        }),
        budget: Budget::Coverage(coverage_budget),
        offenders: Offenders::Coverage(offenders),
        delta_vs_previous: None,
        delta_vs_baseline: None,
    })
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

fn format_command(program: &str, args: &[String]) -> String {
    if args.is_empty() {
        program.to_string()
    } else {
        format!("{program} {}", args.join(" "))
    }
}
