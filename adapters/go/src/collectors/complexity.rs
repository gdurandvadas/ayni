use super::util::run_tool_for_context;
use ayni_adapters_common::collector::{CollectorError, CollectorResult};
use ayni_adapters_common::failure::command_failure_from_output;
use ayni_adapters_common::paths::{resolve_repo_path, to_repo_relative_path};
use ayni_core::{
    Budget, ComplexityBudget, ComplexityOffender, ComplexityResult, FloatThresholdBudget, Language,
    Level, Offenders, RunContext, SignalKind, SignalResult, SignalRow, classify_maximum,
};
use regex::Regex;
use std::path::Path;

pub fn collect(context: &RunContext) -> CollectorResult {
    let config =
        context.policy.go.complexity.as_ref().ok_or_else(|| {
            CollectorError::Adapter(String::from("missing [go.complexity] policy"))
        })?;
    let cyclomatic = config.fn_cyclomatic.ok_or_else(|| {
        CollectorError::Adapter(String::from("missing go.complexity.fn_cyclomatic"))
    })?;

    let args = complexity_args(context);
    let output = run_tool_for_context(context, "gocyclo", &args)?;
    let status_ok = output.status.success();
    let stdout = String::from_utf8_lossy(&output.stdout);

    let re = Regex::new(r"^(\d+)\s+(\S+)\s+(\S+)\s+(.+):(\d+):\d+$").map_err(|error| {
        CollectorError::Adapter(format!("failed to compile gocyclo parser regex: {error}"))
    })?;

    let mut offenders = Vec::<ComplexityOffender>::new();
    let mut measured_functions = 0_u64;
    let mut max_fn_cyclomatic = 0.0_f64;
    let mut warn_count = 0_u64;
    let mut fail_count = 0_u64;

    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Some(caps) = re.captures(trimmed) else {
            continue;
        };
        let complexity = caps
            .get(1)
            .and_then(|value| value.as_str().parse::<f64>().ok())
            .unwrap_or(0.0);
        let function = caps
            .get(3)
            .map(|value| value.as_str().to_string())
            .unwrap_or_else(|| String::from("<function>"));
        let raw_file = caps
            .get(4)
            .map(|value| value.as_str())
            .unwrap_or("<unknown>");
        let line_number = caps
            .get(5)
            .and_then(|value| value.as_str().parse::<u64>().ok())
            .unwrap_or(1);
        let file = to_repo_relative_path(
            &context.repo_root,
            &resolve_reported_path(context, raw_file),
        );

        measured_functions += 1;
        max_fn_cyclomatic = max_fn_cyclomatic.max(complexity);

        let level = classify_complexity(
            complexity,
            cyclomatic.warn,
            cyclomatic.fail,
            &mut warn_count,
            &mut fail_count,
        );

        if let Some(level) = level {
            offenders.push(ComplexityOffender {
                file,
                line: line_number,
                function,
                cyclomatic: complexity,
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
            .then_with(|| left.line.cmp(&right.line))
    });

    let budget = ComplexityBudget {
        fn_cyclomatic: Some(FloatThresholdBudget {
            warn: cyclomatic.warn,
            fail: cyclomatic.fail,
        }),
        fn_cognitive: config.fn_cognitive.map(|threshold| FloatThresholdBudget {
            warn: threshold.warn,
            fail: threshold.fail,
        }),
    };

    Ok(SignalRow {
        kind: SignalKind::Complexity,
        language: Language::Go,
        scope: context.scope.clone(),
        pass: status_ok && fail_count == 0,
        result: SignalResult::Complexity(ComplexityResult {
            engine: String::from("gocyclo"),
            method: String::from("cyclomatic"),
            measured_functions,
            max_fn_cyclomatic,
            max_fn_cognitive: None,
            warn_count,
            fail_count,
            failure: (!status_ok).then(|| {
                command_failure_from_output(
                    context,
                    SignalKind::Complexity,
                    "gocyclo",
                    &args,
                    &output,
                )
            }),
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

fn complexity_args(context: &RunContext) -> Vec<String> {
    context.scope.file.as_ref().map_or_else(
        || vec![String::from(".")],
        |file| {
            vec![
                resolve_repo_path(&context.repo_root, file)
                    .to_string_lossy()
                    .into_owned(),
            ]
        },
    )
}

fn resolve_reported_path(context: &RunContext, raw_file: &str) -> std::path::PathBuf {
    let path = Path::new(raw_file);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        context.execution.exec_cwd.join(path)
    }
}

#[cfg(test)]
mod tests {
    use super::{classify_complexity, complexity_args};
    use ayni_core::{AyniPolicy, ExecutionResolution, Level, RunContext, Scope};
    use std::path::PathBuf;

    #[test]
    fn file_scope_is_passed_as_the_only_gocyclo_target() {
        let context = RunContext {
            repo_root: PathBuf::from("/repo"),
            target_root: PathBuf::from("/repo"),
            workdir: PathBuf::from("/repo"),
            policy: AyniPolicy::default(),
            scope: Scope {
                file: Some(String::from("internal/api/handler.go")),
                ..Scope::default()
            },
            execution: ExecutionResolution::direct("go", PathBuf::from("/repo"), "go.mod", 100),
            cancellation: Default::default(),
            debug: false,
        };

        assert_eq!(
            complexity_args(&context),
            vec![String::from("/repo/internal/api/handler.go")]
        );
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
