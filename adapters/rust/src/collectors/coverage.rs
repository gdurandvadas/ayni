use ayni_adapters_common::collector::CollectorResult;
use ayni_adapters_common::exec::{format_command, run_command_for_context_structured};
use ayni_adapters_common::failure::{concise_failure_message, coverage_metric_failure};
use ayni_core::{
    Budget, CommandFailure, ConfiguredMetricEvaluation, CoverageOffender, CoveragePolicy,
    CoverageResult, Level, Offenders, RunContext, Scope, SignalKind, SignalResult, SignalRow,
    evaluate_configured_metric,
};
use serde_json::{Value as JsonValue, json};

pub fn collect(context: &RunContext) -> CollectorResult {
    let (program, args, engine_label) = coverage_command(context);
    let output = run_command_for_context_structured(context, &program, &args)?;

    let (status, raw_line_percent, raw_branch_percent, report_failure) = if output.status.success()
    {
        match serde_json::from_slice::<JsonValue>(&output.stdout) {
            Ok(payload) => {
                let (line, branch) = find_coverage_percents(&payload);
                (String::from("ok"), line, branch, None)
            }
            Err(error) => (
                String::from("error"),
                Some(f64::NAN),
                Some(f64::NAN),
                Some(CommandFailure {
                    category: String::from("repo_setup_issue"),
                    classification: String::from("unparseable_coverage_report"),
                    command: engine_label.clone(),
                    cwd: context.execution.exec_cwd.display().to_string(),
                    exit_code: None,
                    message: format!("failed to parse cargo llvm-cov coverage report: {error}"),
                }),
            ),
        }
    } else {
        (String::from("error"), None, None, None)
    };
    let line_percent = finite_percent(raw_line_percent);
    let branch_percent = finite_percent(raw_branch_percent);
    let percent = line_percent.or(branch_percent);

    let coverage_config = context.policy.rust.coverage.as_ref();
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

    let assessment = assess_coverage(raw_line_percent, raw_branch_percent, coverage_config);
    let metric_failure = coverage_metric_failure(
        context,
        engine_label.clone(),
        "line_percent",
        assessment.line,
    )
    .or_else(|| {
        coverage_metric_failure(
            context,
            engine_label.clone(),
            "branch_percent",
            assessment.branch,
        )
    });
    let pass = status == "ok" && metric_failure.is_none() && !assessment.has_fail;

    Ok(SignalRow {
        kind: SignalKind::Coverage,
        language: ayni_core::Language::Rust,
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
            branch_percent,
            engine: engine_label,
            status,
            failure: (!output.status.success())
                .then(|| command_failure(context, &program, &args, &output, "repo_code_issue"))
                .or(report_failure)
                .or(metric_failure),
        }),
        budget: Budget::Coverage(coverage_budget),
        offenders: Offenders::Coverage(assessment.offenders),
    })
}

fn command_failure(
    context: &RunContext,
    program: &str,
    args: &[String],
    output: &std::process::Output,
    category: &str,
) -> CommandFailure {
    CommandFailure {
        category: category.to_string(),
        classification: String::from("command_error"),
        command: format_command(program, args),
        cwd: context.execution.exec_cwd.display().to_string(),
        exit_code: output.status.code(),
        message: concise_failure_message(output),
    }
}

fn coverage_command(context: &RunContext) -> (String, Vec<String>, String) {
    if let Some(override_cmd) = context
        .policy
        .tool_override_for(ayni_core::Language::Rust, SignalKind::Coverage)
    {
        let args = if override_cmd.args.is_empty() {
            vec![
                String::from("llvm-cov"),
                String::from("--workspace"),
                String::from("--json"),
                String::from("--summary-only"),
            ]
        } else {
            override_cmd.args.clone()
        };
        let engine = format_command(&override_cmd.command, &args);
        return (override_cmd.command.clone(), args, engine);
    }
    (
        String::from("cargo"),
        vec![
            String::from("llvm-cov"),
            String::from("--workspace"),
            String::from("--json"),
            String::from("--summary-only"),
        ],
        String::from("cargo-llvm-cov"),
    )
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
                file: String::from("<workspace>"),
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

fn find_coverage_percents(value: &JsonValue) -> (Option<f64>, Option<f64>) {
    // `cargo llvm-cov --json --summary-only` puts workspace rollups in `data[0].totals`.
    // Recursing the tree visits `files` before `totals`, so we must read totals first.
    if let Some(data) = value.get("data").and_then(JsonValue::as_array)
        && let Some(first) = data.first()
        && let Some(totals) = first.get("totals").and_then(JsonValue::as_object)
    {
        let line = percent_from_summary_bucket(totals, "lines");
        let branch = percent_from_summary_bucket(totals, "branches");
        return (line, branch);
    }

    let mut line_percent = None;
    let mut branch_percent = None;
    collect_coverage_percents(value, &mut line_percent, &mut branch_percent);
    (line_percent, branch_percent)
}

fn percent_from_summary_bucket(
    map: &serde_json::Map<String, JsonValue>,
    bucket: &str,
) -> Option<f64> {
    map.get(bucket)
        .and_then(JsonValue::as_object)
        .and_then(|summary| summary.get("percent"))
        .map(|percent| percent.as_f64().unwrap_or(f64::NAN))
}

fn collect_coverage_percents(
    value: &JsonValue,
    line_percent: &mut Option<f64>,
    branch_percent: &mut Option<f64>,
) {
    match value {
        JsonValue::Object(map) => {
            if line_percent.is_none() {
                // cargo-llvm-cov uses `lines.percent` / `branches.percent` (see `totals`, per-file `summary`).
                *line_percent =
                    read_percent(map, &["line_percent", "lines", "line"], &["percent", "pct"]);
            }
            if branch_percent.is_none() {
                *branch_percent = read_percent(
                    map,
                    &["branch_percent", "branches", "branch"],
                    &["percent", "pct"],
                );
            }
            for nested in map.values() {
                if line_percent.is_some() && branch_percent.is_some() {
                    return;
                }
                collect_coverage_percents(nested, line_percent, branch_percent);
            }
        }
        JsonValue::Array(items) => {
            for item in items {
                if line_percent.is_some() && branch_percent.is_some() {
                    return;
                }
                collect_coverage_percents(item, line_percent, branch_percent);
            }
        }
        _ => {}
    }
}

fn read_percent(
    map: &serde_json::Map<String, JsonValue>,
    direct_keys: &[&str],
    nested_keys: &[&str],
) -> Option<f64> {
    for key in direct_keys {
        if let Some(value) = map.get(*key) {
            if value.is_number() {
                return Some(value.as_f64().unwrap_or(f64::NAN));
            }
            if let Some(obj) = value.as_object() {
                for nested in nested_keys {
                    if let Some(value) = obj.get(*nested) {
                        return Some(value.as_f64().unwrap_or(f64::NAN));
                    }
                }
            } else {
                return Some(f64::NAN);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{assess_coverage, coverage_command, find_coverage_percents};
    use ayni_core::{
        AyniPolicy, ConfiguredMetricEvaluation, CoveragePolicy, ExecutionResolution, Level,
        RunContext, Scope, ThresholdFloat,
    };
    use serde_json::json;
    use std::path::PathBuf;

    fn context_with_policy(document: &str) -> RunContext {
        let policy: AyniPolicy = toml::from_str(document).expect("policy");
        RunContext {
            repo_root: PathBuf::from("."),
            target_root: PathBuf::from("."),
            workdir: PathBuf::from("."),
            policy,
            scope: Scope::default(),
            execution: ExecutionResolution::direct("cargo", PathBuf::from("."), "test", 100),
            debug: false,
        }
    }

    #[test]
    fn parses_workspace_totals_not_first_file() {
        let payload = json!({
          "data": [{
            "files": [{
              "filename": "/x.rs",
              "summary": {
                "lines": { "percent": 0.0 },
                "branches": { "percent": 0.0 }
              }
            }],
            "totals": {
              "lines": { "percent": 42.5 },
              "branches": { "percent": 12.25 }
            }
          }]
        });
        let (line, branch) = find_coverage_percents(&payload);
        assert_eq!(line, Some(42.5));
        assert_eq!(branch, Some(12.25));
    }

    fn policy(warn: f64, fail: f64) -> CoveragePolicy {
        CoveragePolicy {
            line_percent: Some(ThresholdFloat { warn, fail }),
            branch_percent: None,
        }
    }

    #[test]
    fn enforces_line_and_branch_threshold_boundaries() {
        let policy = CoveragePolicy {
            line_percent: Some(ThresholdFloat {
                warn: 80.0,
                fail: 70.0,
            }),
            branch_percent: Some(ThresholdFloat {
                warn: 60.0,
                fail: 50.0,
            }),
        };
        let equal_warn = assess_coverage(Some(80.0), Some(60.0), Some(&policy));
        assert!(equal_warn.offenders.is_empty());
        assert!(!equal_warn.has_fail);

        let equal_fail = assess_coverage(Some(70.0), Some(50.0), Some(&policy));
        assert_eq!(equal_fail.offenders.len(), 2);
        assert!(
            equal_fail
                .offenders
                .iter()
                .all(|offender| offender.level == Level::Warn)
        );

        let below_fail = assess_coverage(Some(69.0), Some(49.0), Some(&policy));
        assert!(below_fail.has_fail);
        assert!(
            below_fail
                .offenders
                .iter()
                .all(|offender| offender.level == Level::Fail)
        );
    }

    #[test]
    fn preserves_zero_and_rejects_missing_or_non_finite_configured_evidence() {
        let configured = policy(70.0, 50.0);
        let zero = assess_coverage(Some(0.0), None, Some(&configured));
        assert_eq!(zero.offenders[0].value, 0.0);
        assert!(zero.has_fail);
        assert!(matches!(
            assess_coverage(None, None, Some(&configured)).line,
            ConfiguredMetricEvaluation::Missing
        ));
        assert!(matches!(
            assess_coverage(Some(f64::NAN), None, Some(&configured)).line,
            ConfiguredMetricEvaluation::Unparseable
        ));
    }

    #[test]
    fn default_coverage_command_is_cargo_llvm_cov() {
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
enabled = ["rust"]
"#,
        );
        let (program, args, engine) = coverage_command(&context);
        assert_eq!(program, "cargo");
        assert_eq!(
            args,
            vec!["llvm-cov", "--workspace", "--json", "--summary-only"]
        );
        assert_eq!(engine, "cargo-llvm-cov");
    }

    #[test]
    fn coverage_command_uses_rust_tooling_override() {
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
enabled = ["rust"]

[rust.tooling.coverage]
command = "cargo"
args = ["llvm-cov", "--json"]
"#,
        );
        let (program, args, engine) = coverage_command(&context);
        assert_eq!(program, "cargo");
        assert_eq!(args, vec!["llvm-cov", "--json"]);
        assert_eq!(engine, "cargo llvm-cov --json");
    }
}
