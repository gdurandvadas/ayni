use ayni_core::{
    Budget, ComplexityOffender, ComplexityResult, Language, Level, Offenders, RunContext, Scope,
    SignalKind, SignalResult, SignalRow,
};
use serde_json::{Map, Value, json};
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn collect(context: &RunContext) -> Result<SignalRow, String> {
    let config = context
        .policy
        .rust
        .complexity
        .as_ref()
        .ok_or_else(|| String::from("missing [rust.complexity] policy"))?;
    let cyclomatic = config
        .fn_cyclomatic
        .ok_or_else(|| String::from("missing rust.complexity.fn_cyclomatic"))?;

    let target = resolve_analysis_target(context)?;
    let metrics = run_rust_code_analysis(&context.repo_root, &context.workdir, &target)?;

    let mut offenders = Vec::new();
    let mut measured_functions = 0_u64;
    let mut max_fn_cyclomatic = 0.0_f64;
    let mut max_fn_cognitive = None::<f64>;
    let mut warn_count = 0_u64;
    let mut fail_count = 0_u64;

    for metric in metrics {
        measured_functions += 1;
        max_fn_cyclomatic = max_fn_cyclomatic.max(metric.cyclomatic);
        if let Some(cognitive) = metric.cognitive {
            max_fn_cognitive = Some(max_fn_cognitive.unwrap_or(0.0).max(cognitive));
        }

        let level = max_level(
            threshold_level(metric.cyclomatic, cyclomatic.warn, cyclomatic.fail),
            metric.cognitive.and_then(|value| {
                config
                    .fn_cognitive
                    .and_then(|t| threshold_level(value, t.warn, t.fail))
            }),
        );

        if let Some(level) = level {
            match level {
                Level::Warn => warn_count += 1,
                Level::Fail => fail_count += 1,
            }
            offenders.push(ComplexityOffender {
                file: metric.file,
                line: metric.line,
                function: metric.function,
                cyclomatic: round2(metric.cyclomatic),
                cognitive: metric.cognitive.map(round2),
                level,
            });
        }
    }

    offenders.sort_by(|left, right| {
        level_rank(right.level)
            .cmp(&level_rank(left.level))
            .then_with(|| right.cyclomatic.total_cmp(&left.cyclomatic))
            .then_with(|| left.file.cmp(&right.file))
            .then_with(|| left.line.cmp(&right.line))
            .then_with(|| left.function.cmp(&right.function))
    });

    let mut budget = json!({
        "fn_cyclomatic": {"warn": cyclomatic.warn, "fail": cyclomatic.fail},
    });
    if let Some(cognitive) = config.fn_cognitive
        && let Some(map) = budget.as_object_mut()
    {
        map.insert(
            String::from("fn_cognitive"),
            json!({"warn": cognitive.warn, "fail": cognitive.fail}),
        );
    }

    Ok(SignalRow {
        kind: SignalKind::Complexity,
        language: Language::Rust,
        scope: Scope {
            workspace_root: context.scope.workspace_root.clone(),
            path: context.scope.path.clone(),
            package: context.scope.package.clone(),
            file: context.scope.file.clone(),
        },
        pass: fail_count == 0,
        result: SignalResult::Complexity(ComplexityResult {
            engine: String::from("rust-code-analysis-cli"),
            method: String::from("ast_metrics"),
            measured_functions,
            max_fn_cyclomatic: round2(max_fn_cyclomatic),
            max_fn_cognitive: max_fn_cognitive.map(round2),
            warn_count,
            fail_count,
        }),
        budget: Budget::Complexity(budget),
        offenders: Offenders::Complexity(offenders),
        delta_vs_previous: None,
        delta_vs_baseline: None,
    })
}

#[derive(Debug, Clone, PartialEq)]
struct FunctionMetric {
    file: String,
    line: u64,
    function: String,
    cyclomatic: f64,
    cognitive: Option<f64>,
}

#[derive(Debug, serde::Deserialize)]
struct MetadataPackage {
    name: String,
    manifest_path: String,
}

#[derive(Debug, serde::Deserialize)]
struct CargoMetadata {
    packages: Vec<MetadataPackage>,
}

fn resolve_analysis_target(context: &RunContext) -> Result<PathBuf, String> {
    let target = if let Some(file) = &context.scope.file {
        resolve_repo_path(&context.repo_root, file)
    } else if let Some(path) = &context.scope.path {
        resolve_repo_path(&context.repo_root, path)
    } else if let Some(package) = &context.scope.package {
        resolve_package_path(&context.workdir, package)?
    } else {
        context.workdir.clone()
    };

    target.canonicalize().map_err(|error| {
        format!(
            "complexity scope {} could not be resolved: {error}",
            target.display()
        )
    })
}

fn resolve_repo_path(repo_root: &Path, value: &str) -> PathBuf {
    let path = Path::new(value);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        repo_root.join(path)
    }
}

fn resolve_package_path(repo_root: &Path, package: &str) -> Result<PathBuf, String> {
    let metadata = load_metadata(repo_root)?;
    metadata
        .packages
        .into_iter()
        .find(|candidate| candidate.name == package)
        .and_then(|candidate| {
            PathBuf::from(candidate.manifest_path)
                .parent()
                .map(Path::to_path_buf)
        })
        .ok_or_else(|| format!("package scope '{package}' was not found in cargo metadata"))
}

fn load_metadata(repo_root: &Path) -> Result<CargoMetadata, String> {
    let output = Command::new("cargo")
        .arg("metadata")
        .arg("--format-version")
        .arg("1")
        .arg("--no-deps")
        .current_dir(repo_root)
        .output()
        .map_err(|error| format!("failed to execute cargo metadata: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "cargo metadata failed (exit {}): {}",
            output.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("failed to parse cargo metadata output: {error}"))
}

fn run_rust_code_analysis(
    repo_root: &Path,
    workdir: &Path,
    target: &Path,
) -> Result<Vec<FunctionMetric>, String> {
    let mut command = Command::new("rust-code-analysis-cli");
    command
        .arg("--metrics")
        .arg("--paths")
        .arg(target)
        .arg("--language-type")
        .arg("rust")
        .arg("--output-format")
        .arg("json")
        .current_dir(workdir);
    if target.is_dir() {
        command.arg("--include").arg("*.rs");
    }

    let output = command.output().map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            String::from(
                "rust-code-analysis-cli is not installed; run `cargo install rust-code-analysis-cli`",
            )
        } else {
            format!("failed to execute rust-code-analysis-cli: {error}")
        }
    })?;
    if !output.status.success() {
        return Err(format!(
            "rust-code-analysis-cli failed (exit {}): {}",
            output.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    let canonical_repo_root = repo_root
        .canonicalize()
        .unwrap_or_else(|_| repo_root.to_path_buf());
    parse_rust_code_analysis_output(
        &String::from_utf8_lossy(&output.stdout),
        &canonical_repo_root,
        workdir,
    )
}

fn parse_rust_code_analysis_output(
    stdout: &str,
    repo_root: &Path,
    workdir: &Path,
) -> Result<Vec<FunctionMetric>, String> {
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return Err(String::from(
            "rust-code-analysis-cli produced empty output; check the selected scope and tool installation",
        ));
    }

    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        let mut metrics = Vec::new();
        walk_metric_tree(&value, repo_root, workdir, None, &mut metrics);
        if metrics.is_empty() {
            return Err(String::from(
                "rust-code-analysis-cli output was valid JSON but did not contain function metrics",
            ));
        }
        return Ok(metrics);
    }

    let mut metrics = Vec::new();
    let mut parsed_lines = 0_u64;
    for line in trimmed.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let value: Value = serde_json::from_str(line).map_err(|error| {
            format!("failed to parse rust-code-analysis-cli JSON line: {error}")
        })?;
        parsed_lines += 1;
        walk_metric_tree(&value, repo_root, workdir, None, &mut metrics);
    }

    if parsed_lines == 0 {
        return Err(String::from(
            "rust-code-analysis-cli output was neither JSON nor NDJSON",
        ));
    }
    if metrics.is_empty() {
        return Err(String::from(
            "rust-code-analysis-cli output did not contain function metrics",
        ));
    }
    Ok(metrics)
}

fn walk_metric_tree(
    value: &Value,
    repo_root: &Path,
    workdir: &Path,
    file_hint: Option<&str>,
    out: &mut Vec<FunctionMetric>,
) {
    match value {
        Value::Object(map) => {
            let kind = map.get("kind").and_then(Value::as_str);
            let file_from_unit = if kind == Some("unit") {
                map.get("name")
                    .and_then(Value::as_str)
                    .filter(|name| name.contains('/') || name.ends_with(".rs"))
                    .map(|path| repo_relative_metric_path(repo_root, workdir, path))
            } else {
                None
            };
            let effective_file = file_from_unit.as_deref().or(file_hint);

            if kind == Some("function")
                && let Some(metric) = parse_function_metric(map, repo_root, workdir, effective_file)
            {
                out.push(metric);
            }

            if let Some(spaces) = map.get("spaces").and_then(Value::as_array) {
                for child in spaces {
                    walk_metric_tree(child, repo_root, workdir, effective_file, out);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                walk_metric_tree(item, repo_root, workdir, file_hint, out);
            }
        }
        _ => {}
    }
}

fn parse_function_metric(
    map: &Map<String, Value>,
    repo_root: &Path,
    workdir: &Path,
    file_fallback: Option<&str>,
) -> Option<FunctionMetric> {
    let metrics = map.get("metrics")?.as_object()?;
    let cyclomatic = metric_aggregate(metrics, &["cyclomatic", "cyclomatic_complexity"])?;
    let cognitive = metric_aggregate(metrics, &["cognitive", "cognitive_complexity"]);
    let file = metric_string(map, &["path", "file", "filepath"])
        .map(|path| repo_relative_metric_path(repo_root, workdir, &path))
        .or_else(|| {
            file_fallback.map(|path| repo_relative_metric_path(repo_root, workdir, path))
        })?;
    let function = metric_string(map, &["name", "function", "function_name"])?;
    let line = metric_u64(map, &["start_line", "line", "begin_line"]).unwrap_or(1);
    Some(FunctionMetric {
        file,
        line,
        function,
        cyclomatic,
        cognitive,
    })
}

fn repo_relative_metric_path(repo_root: &Path, workdir: &Path, path: &str) -> String {
    let normalized = path.replace('\\', "/");
    let candidate = Path::new(&normalized);
    if candidate.is_absolute()
        && let Ok(relative) = candidate.strip_prefix(repo_root)
    {
        return relative.to_string_lossy().replace('\\', "/");
    }
    let from_workdir = workdir.join(candidate);
    if let Ok(relative) = from_workdir.strip_prefix(repo_root) {
        return relative.to_string_lossy().replace('\\', "/");
    }
    let from_repo = repo_root.join(candidate);
    if let Ok(relative) = from_repo.strip_prefix(repo_root) {
        return relative.to_string_lossy().replace('\\', "/");
    }
    normalized.trim_start_matches("./").to_string()
}

fn metric_aggregate(map: &Map<String, Value>, keys: &[&str]) -> Option<f64> {
    for key in keys {
        match map.get(*key) {
            Some(Value::Number(number)) => return number.as_f64(),
            Some(Value::Object(obj)) => {
                if let Some(value) = obj.get("max").and_then(Value::as_f64) {
                    return Some(value);
                }
                if let Some(value) = obj.get("sum").and_then(Value::as_f64) {
                    return Some(value);
                }
            }
            _ => {}
        }
    }
    None
}

fn metric_u64(map: &Map<String, Value>, keys: &[&str]) -> Option<u64> {
    for key in keys {
        if let Some(value) = map.get(*key).and_then(Value::as_u64) {
            return Some(value);
        }
    }
    None
}

fn metric_string(map: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(value) = map.get(*key).and_then(Value::as_str) {
            return Some(String::from(value));
        }
    }
    None
}

fn threshold_level(value: f64, warn: f64, fail: f64) -> Option<Level> {
    if value > fail {
        Some(Level::Fail)
    } else if value > warn {
        Some(Level::Warn)
    } else {
        None
    }
}

fn max_level(left: Option<Level>, right: Option<Level>) -> Option<Level> {
    match (left, right) {
        (Some(Level::Fail), _) | (_, Some(Level::Fail)) => Some(Level::Fail),
        (Some(Level::Warn), _) | (_, Some(Level::Warn)) => Some(Level::Warn),
        _ => None,
    }
}

fn round2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

fn level_rank(level: Level) -> u8 {
    match level {
        Level::Warn => 1,
        Level::Fail => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_function_metric, parse_rust_code_analysis_output};
    use serde_json::json;
    use tempfile::TempDir;

    #[test]
    fn parse_single_json_document_metrics() {
        let temp = TempDir::new().expect("tempdir");
        let root = temp.path();
        let payload = json!({
            "kind": "unit",
            "name": format!("{}/core/src/lib.rs", root.display()),
            "spaces": [{
                "kind": "function",
                "name": "alpha",
                "start_line": 12,
                "metrics": {
                    "cyclomatic": { "max": 11.0 },
                    "cognitive": { "max": 7.0 }
                }
            }]
        });

        let metrics =
            parse_rust_code_analysis_output(&payload.to_string(), root, root).expect("metrics");
        assert_eq!(metrics.len(), 1);
        assert_eq!(metrics[0].file, "core/src/lib.rs");
        assert_eq!(metrics[0].function, "alpha");
        assert_eq!(metrics[0].line, 12);
        assert_eq!(metrics[0].cyclomatic, 11.0);
        assert_eq!(metrics[0].cognitive, Some(7.0));
    }

    #[test]
    fn parse_ndjson_metrics() {
        let temp = TempDir::new().expect("tempdir");
        let root = temp.path();
        let first = json!({
            "kind": "unit",
            "name": format!("{}/core/src/lib.rs", root.display()),
            "spaces": [{
                "kind": "function",
                "name": "alpha",
                "start_line": 10,
                "metrics": { "cyclomatic": { "max": 5.0 } }
            }]
        });
        let second = json!({
            "kind": "unit",
            "name": format!("{}/cli/src/main.rs", root.display()),
            "spaces": [{
                "kind": "function",
                "name": "beta",
                "start_line": 20,
                "metrics": {
                    "cyclomatic": { "max": 13.0 },
                    "cognitive_complexity": { "max": 9.0 }
                }
            }]
        });
        let payload = format!("{}\n{}\n", first, second);

        let metrics = parse_rust_code_analysis_output(&payload, root, root).expect("metrics");
        assert_eq!(metrics.len(), 2);
        assert_eq!(metrics[1].file, "cli/src/main.rs");
        assert_eq!(metrics[1].function, "beta");
        assert_eq!(metrics[1].cognitive, Some(9.0));
    }

    #[test]
    fn parse_function_metric_supports_direct_path_fields() {
        let temp = TempDir::new().expect("tempdir");
        let root = temp.path();
        let value = json!({
            "path": "core/src/lib.rs",
            "name": "example_fn",
            "start_line": 42,
            "metrics": {
                "cyclomatic": { "max": 12.0, "sum": 12.0 },
                "cognitive": { "max": 7.0, "sum": 7.0 }
            }
        });
        let map = value.as_object().expect("object");
        let metric = parse_function_metric(map, root, root, None).expect("metric");

        assert_eq!(metric.file, "core/src/lib.rs");
        assert_eq!(metric.function, "example_fn");
        assert_eq!(metric.line, 42);
        assert_eq!(metric.cyclomatic, 12.0);
        assert_eq!(metric.cognitive, Some(7.0));
    }
}
