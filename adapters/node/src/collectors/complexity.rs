use super::util::{run_tool, to_repo_relative_path};
use ayni_core::{
    Budget, ComplexityOffender, ComplexityResult, Language, Level, Offenders, RunContext, Scope,
    SignalKind, SignalResult, SignalRow,
};
use regex::Regex;
use serde_json::Value as JsonValue;
use serde_json::json;

pub fn collect(context: &RunContext) -> Result<SignalRow, String> {
    let config = context
        .policy
        .node
        .complexity
        .as_ref()
        .ok_or_else(|| String::from("missing [node.complexity] policy"))?;
    let cyclomatic = config
        .fn_cyclomatic
        .ok_or_else(|| String::from("missing node.complexity.fn_cyclomatic"))?;

    let output = run_tool(
        context,
        "eslint",
        &[
            ".",
            "--format",
            "json",
            "--ext",
            ".ts,.tsx,.js,.jsx,.mjs,.cjs",
            "--no-error-on-unmatched-pattern",
        ],
    )?;
    let stdout_text = String::from_utf8_lossy(&output.stdout);
    let report = serde_json::from_str::<JsonValue>(&stdout_text).unwrap_or_else(|_| json!([]));
    let entries = report.as_array().cloned().unwrap_or_default();

    let re_complexity = Regex::new(r"complexity of (\d+)").map_err(|e| e.to_string())?;
    let mut offenders = Vec::<ComplexityOffender>::new();
    let mut measured_functions = 0_u64;
    let mut max_fn_cyclomatic = 0.0_f64;
    let mut max_fn_cognitive = None::<f64>;
    let mut warn_count = 0_u64;
    let mut fail_count = 0_u64;

    for item in entries {
        let file = item
            .get("filePath")
            .and_then(JsonValue::as_str)
            .map(|value| to_repo_relative_path(&context.repo_root, Path::new(value)))
            .unwrap_or_else(|| String::from("<unknown>"));
        let Some(messages) = item.get("messages").and_then(JsonValue::as_array) else {
            continue;
        };
        for message in messages {
            let rule_id = message
                .get("ruleId")
                .and_then(JsonValue::as_str)
                .unwrap_or_default();
            if !rule_id.contains("complexity") {
                continue;
            }
            let raw_message = message
                .get("message")
                .and_then(JsonValue::as_str)
                .unwrap_or("complexity threshold violated");
            let complexity_value = re_complexity
                .captures(raw_message)
                .and_then(|caps| caps.get(1))
                .and_then(|m| m.as_str().parse::<f64>().ok())
                .unwrap_or(cyclomatic.fail + 1.0);
            measured_functions += 1;
            max_fn_cyclomatic = max_fn_cyclomatic.max(complexity_value);

            let level = if complexity_value > cyclomatic.fail {
                fail_count += 1;
                Level::Fail
            } else if complexity_value > cyclomatic.warn {
                warn_count += 1;
                Level::Warn
            } else {
                continue;
            };

            offenders.push(ComplexityOffender {
                file: file.clone(),
                line: message.get("line").and_then(JsonValue::as_u64).unwrap_or(1),
                function: message
                    .get("nodeType")
                    .and_then(JsonValue::as_str)
                    .map(String::from)
                    .unwrap_or_else(|| String::from("<function>")),
                cyclomatic: complexity_value,
                cognitive: None,
                level,
            });
        }
    }

    offenders.sort_by(|left, right| {
        level_rank(right.level)
            .cmp(&level_rank(left.level))
            .then_with(|| right.cyclomatic.total_cmp(&left.cyclomatic))
            .then_with(|| left.file.cmp(&right.file))
    });

    let mut budget = json!({
        "fn_cyclomatic": {"warn": cyclomatic.warn, "fail": cyclomatic.fail}
    });
    if let Some(cognitive) = config.fn_cognitive
        && let Some(map) = budget.as_object_mut()
    {
        map.insert(
            String::from("fn_cognitive"),
            json!({"warn": cognitive.warn, "fail": cognitive.fail}),
        );
    }

    let pass = output.status.success() && fail_count == 0;
    Ok(SignalRow {
        kind: SignalKind::Complexity,
        language: Language::Node,
        scope: Scope {
            workspace_root: context.scope.workspace_root.clone(),
            path: context.scope.path.clone(),
            package: context.scope.package.clone(),
            file: context.scope.file.clone(),
        },
        pass,
        result: SignalResult::Complexity(ComplexityResult {
            engine: String::from("eslint"),
            method: String::from("rule_complexity"),
            measured_functions,
            max_fn_cyclomatic,
            max_fn_cognitive: max_fn_cognitive.take(),
            warn_count,
            fail_count,
        }),
        budget: Budget::Complexity(budget),
        offenders: Offenders::Complexity(offenders),
        delta_vs_previous: None,
        delta_vs_baseline: None,
    })
}

fn level_rank(level: Level) -> u8 {
    match level {
        Level::Warn => 1,
        Level::Fail => 2,
    }
}
use std::path::Path;
