use super::util::{command_failure_from_output, run_tool, tool_command};
use ayni_adapters_common::collector::CollectorResult;
use ayni_adapters_common::exec::{format_command, run_command_for_context_structured};
use ayni_adapters_common::failure::coverage_metric_failure;
use ayni_adapters_common::paths::to_repo_relative_path;
use ayni_core::{
    Budget, ConfiguredMetricEvaluation, CoverageBudget, CoverageOffender, CoveragePolicy,
    CoverageResult, Level, Offenders, RunContext, Scope, SignalKind, SignalResult, SignalRow,
    evaluate_configured_metric,
};
use serde_json::Value as JsonValue;
use std::fs;

pub fn collect(context: &RunContext) -> CollectorResult {
    let (output, engine) = if let Some((program, args, engine)) = coverage_override_command(context)
    {
        (
            run_command_for_context_structured(context, &program, &args)?,
            engine,
        )
    } else {
        let (program, args) = tool_command(
            context,
            "vitest",
            &[
                "run",
                "--coverage",
                "--coverage.reporter=json-summary",
                "--passWithNoTests",
            ],
        );
        let engine = format_command(&program, &args);
        (
            run_tool(
                context,
                "vitest",
                &[
                    "run",
                    "--coverage",
                    "--coverage.reporter=json-summary",
                    "--passWithNoTests",
                ],
            )?,
            engine,
        )
    };
    let coverage_path = context
        .workdir
        .join("coverage")
        .join("coverage-summary.json");
    let summary = fs::read_to_string(&coverage_path)
        .ok()
        .map(|content| serde_json::from_str::<JsonValue>(&content));

    let status = if output.status.success() && matches!(summary, Some(Ok(_))) {
        String::from("ok")
    } else {
        String::from("error")
    };
    let failure = (!output.status.success()).then(|| {
        command_failure_from_output(
            context,
            SignalKind::Coverage,
            engine.split_whitespace().next().unwrap_or("node"),
            &engine
                .split_whitespace()
                .skip(1)
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            &output,
        )
    });
    let (raw_line_percent, raw_branch_percent) = summary
        .as_ref()
        .map(|report| {
            report
                .as_ref()
                .map(find_coverage_percents)
                .unwrap_or((Some(f64::NAN), Some(f64::NAN)))
        })
        .unwrap_or((None, None));
    let line_percent = finite_percent(raw_line_percent);
    let branch_percent = finite_percent(raw_branch_percent);
    let percent = line_percent.or(branch_percent);

    let coverage_config = context.policy.node.coverage.as_ref();
    let coverage_budget = coverage_config
        .map(|config| CoverageBudget {
            line_percent_warn: config.line_percent.map(|value| value.warn),
            line_percent_fail: config.line_percent.map(|value| value.fail),
            branch_percent_warn: config.branch_percent.map(|value| value.warn),
            branch_percent_fail: config.branch_percent.map(|value| value.fail),
        })
        .unwrap_or_default();

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
        language: ayni_core::Language::Node,
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
            engine,
            status,
            failure: failure.or(metric_failure),
        }),
        budget: Budget::Coverage(coverage_budget),
        offenders: Offenders::Coverage(assessment.offenders),
    })
}

fn coverage_override_command(context: &RunContext) -> Option<(String, Vec<String>, String)> {
    let override_cmd = context
        .policy
        .tool_override_for(ayni_core::Language::Node, SignalKind::Coverage)?;
    let args = if override_cmd.args.is_empty() {
        vec![
            String::from("run"),
            String::from("--coverage"),
            String::from("--coverage.reporter=json-summary"),
            String::from("--passWithNoTests"),
        ]
    } else {
        override_cmd.args.clone()
    };
    let engine = format_command(&override_cmd.command, &args);
    Some((override_cmd.command.clone(), args, engine))
}

fn find_coverage_percents(summary: &JsonValue) -> (Option<f64>, Option<f64>) {
    let total = summary.get("total").and_then(JsonValue::as_object);
    (
        total_percent(total, "lines"),
        total_percent(total, "branches"),
    )
}

fn total_percent(total: Option<&serde_json::Map<String, JsonValue>>, metric: &str) -> Option<f64> {
    let value = total?.get(metric)?.as_object()?.get("pct")?;
    Some(value.as_f64().unwrap_or(f64::NAN))
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
    use super::{assess_coverage, coverage_override_command, find_coverage_percents};
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
            execution: ExecutionResolution::direct("npm", PathBuf::from("."), "test", 100),
            cancellation: Default::default(),
            debug: false,
        }
    }

    #[test]
    fn no_override_returns_none() {
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
enabled = ["node"]
"#,
        );
        assert!(coverage_override_command(&context).is_none());
    }

    #[test]
    fn coverage_override_command_uses_node_tooling_override() {
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
enabled = ["node"]

[node.tooling.coverage]
command = "pnpm"
args = ["exec", "vitest", "run", "--coverage"]
"#,
        );
        let (program, args, engine) =
            coverage_override_command(&context).expect("expected node coverage override");
        assert_eq!(program, "pnpm");
        assert_eq!(args, vec!["exec", "vitest", "run", "--coverage"]);
        assert_eq!(engine, "pnpm exec vitest run --coverage");
    }

    #[test]
    fn independently_enforces_line_and_branch_metrics() {
        let context = context_with_policy("");
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
        let equal_warn = assess_coverage(Some(80.0), Some(60.0), Some(&policy), &context);
        assert!(equal_warn.offenders.is_empty());
        let equal_fail = assess_coverage(Some(70.0), Some(50.0), Some(&policy), &context);
        assert!(
            equal_fail
                .offenders
                .iter()
                .all(|offender| offender.level == Level::Warn)
        );
        let below_fail = assess_coverage(Some(69.0), Some(49.0), Some(&policy), &context);
        assert!(below_fail.has_fail);
        assert_eq!(below_fail.offenders.len(), 2);
        let zero = assess_coverage(Some(0.0), Some(0.0), Some(&policy), &context);
        assert!(zero.has_fail);
        assert!(zero.offenders.iter().all(|offender| offender.value == 0.0));
    }

    #[test]
    fn rejects_missing_and_unparseable_required_evidence() {
        let context = context_with_policy("");
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
        let missing = assess_coverage(None, Some(60.0), Some(&policy), &context);
        assert!(matches!(missing.line, ConfiguredMetricEvaluation::Missing));
        let unparseable = assess_coverage(Some(80.0), Some(f64::NAN), Some(&policy), &context);
        assert!(matches!(
            unparseable.branch,
            ConfiguredMetricEvaluation::Unparseable
        ));
    }

    #[test]
    fn parses_native_total_line_and_branch_percentages() {
        let report = json!({"total": {"lines": {"pct": 75.0}, "branches": {"pct": 25.0}}});
        assert_eq!(find_coverage_percents(&report), (Some(75.0), Some(25.0)));
    }
}
