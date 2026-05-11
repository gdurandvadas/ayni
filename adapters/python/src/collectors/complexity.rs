use super::util::{
    command_failure_from_output, format_command, prepare_report_path, run_command_for_context,
    to_repo_relative_path,
};
use ayni_core::{
    Budget, ComplexityOffender, ComplexityResult, Language, Level, Offenders, RunContext, Scope,
    SignalKind, SignalResult, SignalRow,
};
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};

pub fn collect(context: &RunContext) -> Result<SignalRow, String> {
    let config = context
        .policy
        .python
        .complexity
        .as_ref()
        .ok_or_else(|| String::from("missing [python.complexity] policy"))?;
    let cognitive = config
        .fn_cognitive
        .ok_or_else(|| String::from("missing python.complexity.fn_cognitive"))?;

    let report_path = prepare_report_path(context, "complexipy.json")?;
    let threshold = cognitive.fail.to_string();
    let output_path = report_path.to_string_lossy().to_string();
    let args = vec![
        String::from("."),
        String::from("--output-format"),
        String::from("json"),
        String::from("--output"),
        output_path,
        String::from("--max-complexity-allowed"),
        threshold,
        String::from("--ignore-complexity"),
    ];
    let mut command_args = vec![
        String::from("tool"),
        String::from("run"),
        String::from("complexipy"),
    ];
    command_args.extend(args);
    let engine = format_command("uv", &command_args);
    let output = run_command_for_context(context, "uv", &command_args)?;
    if !output.status.success() {
        return Ok(error_row(
            context,
            engine,
            command_failure_from_output(
                context,
                SignalKind::Complexity,
                "uv",
                &command_args,
                &output,
            ),
        ));
    }

    let value = read_report(&report_path)?;
    let mut entries = Vec::new();
    collect_function_entries(&value, None, &mut entries);

    let mut offenders = Vec::new();
    let mut measured_functions = 0_u64;
    let mut max_fn_cognitive = 0.0_f64;
    let mut warn_count = 0_u64;
    let mut fail_count = 0_u64;

    for entry in entries {
        measured_functions += 1;
        max_fn_cognitive = max_fn_cognitive.max(entry.complexity);
        let level = if entry.complexity > cognitive.fail {
            fail_count += 1;
            Some(Level::Fail)
        } else if entry.complexity > cognitive.warn {
            warn_count += 1;
            Some(Level::Warn)
        } else {
            None
        };
        if let Some(level) = level {
            offenders.push(ComplexityOffender {
                file: to_repo_relative_path(
                    &context.repo_root,
                    &resolve_file(context, &entry.file),
                ),
                line: entry.line.unwrap_or(1),
                function: entry.function,
                cyclomatic: 0.0,
                cognitive: Some(entry.complexity),
                level,
            });
        }
    }

    offenders.sort_by(|left, right| {
        level_rank(right.level)
            .cmp(&level_rank(left.level))
            .then_with(|| {
                right
                    .cognitive
                    .unwrap_or(0.0)
                    .total_cmp(&left.cognitive.unwrap_or(0.0))
            })
            .then_with(|| left.file.cmp(&right.file))
            .then_with(|| left.line.cmp(&right.line))
    });

    Ok(SignalRow {
        kind: SignalKind::Complexity,
        language: Language::Python,
        scope: Scope {
            workspace_root: context.scope.workspace_root.clone(),
            path: context.scope.path.clone(),
            package: context.scope.package.clone(),
            file: context.scope.file.clone(),
        },
        pass: fail_count == 0,
        result: SignalResult::Complexity(ComplexityResult {
            engine: String::from("complexipy"),
            failure: None,
            method: String::from("cognitive"),
            measured_functions,
            max_fn_cyclomatic: 0.0,
            max_fn_cognitive: Some(max_fn_cognitive),
            warn_count,
            fail_count,
        }),
        budget: Budget::Complexity(json!({
            "fn_cognitive": {"warn": cognitive.warn, "fail": cognitive.fail}
        })),
        offenders: Offenders::Complexity(offenders),
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
        kind: SignalKind::Complexity,
        language: Language::Python,
        scope: Scope {
            workspace_root: context.scope.workspace_root.clone(),
            path: context.scope.path.clone(),
            package: context.scope.package.clone(),
            file: context.scope.file.clone(),
        },
        pass: false,
        result: SignalResult::Complexity(ComplexityResult {
            engine,
            method: String::from("cognitive"),
            measured_functions: 0,
            max_fn_cyclomatic: 0.0,
            max_fn_cognitive: None,
            warn_count: 0,
            fail_count: 1,
            failure: Some(failure),
        }),
        budget: Budget::Complexity(json!({})),
        offenders: Offenders::Complexity(Vec::new()),
        delta_vs_previous: None,
        delta_vs_baseline: None,
    }
}

#[derive(Debug, Clone)]
struct FunctionEntry {
    file: String,
    function: String,
    line: Option<u64>,
    complexity: f64,
}

fn read_report(path: &Path) -> Result<Value, String> {
    let content = fs::read_to_string(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    serde_json::from_str(&content)
        .map_err(|error| format!("failed to parse {}: {error}", path.display()))
}

fn collect_function_entries(
    value: &Value,
    inherited_file: Option<&str>,
    out: &mut Vec<FunctionEntry>,
) {
    match value {
        Value::Array(values) => {
            for value in values {
                collect_function_entries(value, inherited_file, out);
            }
        }
        Value::Object(map) => {
            let file = string_field(value, &["path", "file", "filename"]).or(inherited_file);
            if let (Some(file), Some(name), Some(complexity)) = (
                file,
                string_field(value, &["name", "function", "function_name"]),
                number_field(value, &["complexity", "cognitive", "score"]),
            ) {
                out.push(FunctionEntry {
                    file: file.to_string(),
                    function: name.to_string(),
                    line: number_field(value, &["line_start", "line", "start_line"])
                        .map(|value| value as u64),
                    complexity,
                });
            }
            for (key, child) in map {
                if !child.is_array() && !child.is_object() {
                    continue;
                }
                let child_file = if matches!(key.as_str(), "functions" | "items" | "results") {
                    file
                } else {
                    inherited_file
                };
                collect_function_entries(child, child_file, out);
            }
        }
        _ => {}
    }
}

fn string_field<'a>(value: &'a Value, names: &[&str]) -> Option<&'a str> {
    names
        .iter()
        .find_map(|name| value.get(*name).and_then(Value::as_str))
}

fn number_field(value: &Value, names: &[&str]) -> Option<f64> {
    names
        .iter()
        .find_map(|name| value.get(*name).and_then(Value::as_f64))
}

fn resolve_file(context: &RunContext, file: &str) -> PathBuf {
    let path = Path::new(file);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        context.workdir.join(path)
    }
}

fn level_rank(level: Level) -> u8 {
    match level {
        Level::Warn => 1,
        Level::Fail => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::{FunctionEntry, collect_function_entries};
    use serde_json::json;

    #[test]
    fn extracts_nested_complexipy_functions() {
        let value = json!([
            {
                "path": "src/app.py",
                "functions": [
                    {"name": "handle", "complexity": 12, "line_start": 4}
                ]
            }
        ]);
        let mut out = Vec::<FunctionEntry>::new();
        collect_function_entries(&value, None, &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].file, "src/app.py");
        assert_eq!(out[0].function, "handle");
        assert_eq!(out[0].line, Some(4));
        assert_eq!(out[0].complexity, 12.0);
    }
}
