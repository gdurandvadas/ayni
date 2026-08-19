use super::util::run_tool;
use ayni_adapters_common::collector::{CollectorError, CollectorResult};
use ayni_adapters_common::failure::setup_failure;
use ayni_adapters_common::paths::to_repo_relative_path;
use ayni_core::{
    Budget, ComplexityBudget, ComplexityOffender, ComplexityResult, FloatThresholdBudget, Language,
    Level, Offenders, RunContext, Scope, SignalKind, SignalResult, SignalRow, classify_maximum,
};
use regex::Regex;
use serde_json::Value as JsonValue;
use std::path::Path;

pub fn collect(context: &RunContext) -> CollectorResult {
    let config =
        context.policy.node.complexity.as_ref().ok_or_else(|| {
            CollectorError::Adapter(String::from("missing [node.complexity] policy"))
        })?;
    let cyclomatic = config.fn_cyclomatic.ok_or_else(|| {
        CollectorError::Adapter(String::from("missing node.complexity.fn_cyclomatic"))
    })?;

    let target = complexity_target(context);
    // Do not inherit a repository rule (or its absence): complexity policy is
    // Ayni-owned evidence. ESLint's flat-config lookup is disabled and the
    // policy warn threshold is injected as an error rule so every function at
    // or above that boundary is reported. A cataloged TypeScript parser is
    // selected explicitly so JavaScript and TypeScript evidence do not depend
    // on repository ESLint configuration.
    let rule = format!(r#"complexity: ["error", {}]"#, cyclomatic.warn);
    let output = run_tool(
        context,
        "eslint",
        &[
            &target,
            "--format",
            "json",
            "--no-config-lookup",
            "--rule",
            &rule,
            "--parser",
            "@typescript-eslint/parser",
            "--ext",
            ".ts,.tsx,.js,.jsx,.mjs,.cjs",
            "--no-error-on-unmatched-pattern",
        ],
    )?;
    let stdout_text = String::from_utf8_lossy(&output.stdout);
    let report = serde_json::from_str::<JsonValue>(&stdout_text).ok();
    let report_missing = report.as_ref().and_then(JsonValue::as_array).is_none();
    let entries = report
        .as_ref()
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();

    let re_complexity =
        Regex::new(r"complexity of (\d+)").map_err(|e| CollectorError::Adapter(e.to_string()))?;
    let mut offenders = Vec::<ComplexityOffender>::new();
    let mut measured_functions = 0_u64;
    let mut max_fn_cyclomatic = 0.0_f64;
    let mut max_fn_cognitive = None::<f64>;
    let mut warn_count = 0_u64;
    let mut fail_count = 0_u64;
    let mut measurement_error = false;

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
                // Parser/configuration errors do not have the complexity rule
                // id. They mean ESLint could not produce policy evidence.
                measurement_error = true;
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

            let Some(level) = classify_complexity(
                complexity_value,
                cyclomatic.warn,
                cyclomatic.fail,
                &mut warn_count,
                &mut fail_count,
            ) else {
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
        right
            .level
            .cmp(&left.level)
            .then_with(|| right.cyclomatic.total_cmp(&left.cyclomatic))
            .then_with(|| left.file.cmp(&right.file))
    });

    let budget = ComplexityBudget {
        fn_cyclomatic: Some(FloatThresholdBudget {
            warn: cyclomatic.warn,
            fail: cyclomatic.fail,
        }),
        fn_cognitive: config.fn_cognitive.map(|cognitive| FloatThresholdBudget {
            warn: cognitive.warn,
            fail: cognitive.fail,
        }),
    };

    // ESLint exits non-zero for the injected warn-boundary rule, so its exit
    // status is not the policy verdict. It remains authoritative for output
    // that cannot be attributed to complexity (parse/configuration failures).
    let unmeasurable = report_missing || measurement_error;
    let pass = fail_count == 0 && !unmeasurable;
    let failure = unmeasurable.then(|| {
        setup_failure(
            context,
            String::from(
                "eslint --no-config-lookup --parser @typescript-eslint/parser --format json --rule complexity",
            ),
            if report_missing {
                "eslint produced no parseable JSON report; complexity cannot be measured"
            } else {
                "eslint reported a parse or configuration error; complexity cannot be measured"
            },
        )
    });
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
            failure,
        }),
        budget: Budget::Complexity(budget),
        offenders: Offenders::Complexity(offenders),
    })
}

fn classify_complexity(
    value: f64,
    warn: f64,
    fail: f64,
    warn_count: &mut u64,
    fail_count: &mut u64,
) -> Option<Level> {
    let level = classify_maximum(value, warn, fail);
    match level {
        Some(Level::Warn) => *warn_count += 1,
        Some(Level::Fail) => *fail_count += 1,
        None => {}
    }
    level
}

fn complexity_target(context: &RunContext) -> String {
    context.scope.file.as_ref().map_or_else(
        || String::from("."),
        |file| {
            ayni_adapters_common::paths::resolve_repo_path(&context.repo_root, file)
                .to_string_lossy()
                .into_owned()
        },
    )
}

#[cfg(test)]
mod tests {
    use super::{classify_complexity, complexity_target};
    use ayni_core::{AyniPolicy, ExecutionResolution, Level, RunContext, Scope};
    use std::path::PathBuf;

    #[test]
    fn file_scope_is_the_eslint_target() {
        let context = RunContext {
            repo_root: PathBuf::from("/repo"),
            target_root: PathBuf::from("/repo"),
            workdir: PathBuf::from("/repo"),
            policy: AyniPolicy::default(),
            scope: Scope {
                file: Some(String::from("src/handler.ts")),
                ..Scope::default()
            },
            execution: ExecutionResolution::direct("npm", PathBuf::from("/repo"), "lock", 100),
            debug: false,
        };

        assert_eq!(complexity_target(&context), "/repo/src/handler.ts");
    }

    #[test]
    fn maximum_threshold_equality() {
        let mut warn_count = 0;
        let mut fail_count = 0;
        assert_eq!(
            classify_complexity(9.0, 10.0, 15.0, &mut warn_count, &mut fail_count),
            None
        );
        assert_eq!(
            classify_complexity(10.0, 10.0, 15.0, &mut warn_count, &mut fail_count),
            Some(Level::Warn)
        );
        assert_eq!(
            classify_complexity(15.0, 10.0, 15.0, &mut warn_count, &mut fail_count),
            Some(Level::Fail)
        );
        assert_eq!((warn_count, fail_count), (1, 1));
    }
}
