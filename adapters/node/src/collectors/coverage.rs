use super::util::{
    command_failure_from_output, run_command_for_context, run_tool, to_repo_relative_path,
};
use ayni_core::{
    Budget, CoverageOffender, CoveragePolicy, CoverageResult, Level, Offenders, RunContext, Scope,
    SignalKind, SignalResult, SignalRow,
};
use serde_json::Value as JsonValue;
use serde_json::json;
use std::fs;

pub fn collect(context: &RunContext) -> Result<SignalRow, String> {
    let (output, engine) = if let Some((program, args, engine)) = coverage_override_command(context)
    {
        (run_command_for_context(context, &program, &args)?, engine)
    } else {
        let output = run_tool(
            context,
            "vitest",
            &[
                "run",
                "--coverage",
                "--coverage.reporter=json-summary",
                "--passWithNoTests",
            ],
        )?;
        (output, String::from("vitest"))
    };
    let coverage_path = context
        .workdir
        .join("coverage")
        .join("coverage-summary.json");
    let summary = fs::read_to_string(&coverage_path)
        .ok()
        .and_then(|content| serde_json::from_str::<JsonValue>(&content).ok());

    let status = if output.status.success() && summary.is_some() {
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
    let (percent, line_percent, branch_percent) = summary
        .as_ref()
        .map(find_coverage_percents)
        .unwrap_or((None, None, None));

    let coverage_config = context.policy.node.coverage.as_ref();
    let coverage_budget = coverage_config
        .map(|config| {
            json!({
                "line_percent_warn": config.line_percent.map(|v| v.warn),
                "line_percent_fail": config.line_percent.map(|v| v.fail),
            })
        })
        .unwrap_or_else(|| json!({}));

    let headline = percent.or(line_percent).or(branch_percent);
    let pass = status == "ok"
        && coverage_config
            .and_then(|c| c.line_percent)
            .is_none_or(|t| headline.is_none_or(|v| v >= t.fail));
    let offenders = build_offenders(headline, coverage_config, context);

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
            failure,
        }),
        budget: Budget::Coverage(coverage_budget),
        offenders: Offenders::Coverage(offenders),
        delta_vs_previous: None,
        delta_vs_baseline: None,
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

fn format_command(program: &str, args: &[String]) -> String {
    if args.is_empty() {
        program.to_string()
    } else {
        format!("{program} {}", args.join(" "))
    }
}

fn find_coverage_percents(summary: &JsonValue) -> (Option<f64>, Option<f64>, Option<f64>) {
    let total = summary.get("total").and_then(JsonValue::as_object);
    let line = total
        .and_then(|v| v.get("lines"))
        .and_then(JsonValue::as_object)
        .and_then(|v| v.get("pct"))
        .and_then(JsonValue::as_f64);
    let branch = total
        .and_then(|v| v.get("branches"))
        .and_then(JsonValue::as_object)
        .and_then(|v| v.get("pct"))
        .and_then(JsonValue::as_f64);
    (line.or(branch), line, branch)
}

fn build_offenders(
    headline: Option<f64>,
    policy: Option<&CoveragePolicy>,
    context: &RunContext,
) -> Vec<CoverageOffender> {
    let Some(value) = headline else {
        return Vec::new();
    };
    let Some(threshold) = policy.and_then(|p| p.line_percent) else {
        return Vec::new();
    };
    if value >= threshold.warn {
        return Vec::new();
    }
    let level = if value < threshold.fail {
        Level::Fail
    } else {
        Level::Warn
    };
    vec![CoverageOffender {
        file: to_repo_relative_path(&context.repo_root, &context.workdir),
        line: None,
        value,
        level,
    }]
}

#[cfg(test)]
mod tests {
    use super::coverage_override_command;
    use ayni_core::{AyniPolicy, ExecutionResolution, RunContext, Scope};
    use std::path::PathBuf;

    fn context_with_policy(document: &str) -> RunContext {
        let policy: AyniPolicy = toml::from_str(document).expect("policy");
        RunContext {
            repo_root: PathBuf::from("."),
            target_root: PathBuf::from("."),
            workdir: PathBuf::from("."),
            policy,
            scope: Scope::default(),
            diff: None,
            execution: ExecutionResolution::direct("npm", PathBuf::from("."), "test", 100),
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
}
