use ayni_adapters_common::collector::{CollectorError, CollectorResult};
use ayni_adapters_common::exec::run_command_for_context_structured;
use ayni_core::{
    Budget, ComplexityBudget, ComplexityOffender, ComplexityResult, FloatThresholdBudget, Language,
    Level, Offenders, RunContext, SignalKind, SignalResult, SignalRow, classify_maximum,
};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

pub fn collect(context: &RunContext) -> CollectorResult {
    let config =
        context.policy.rust.complexity.as_ref().ok_or_else(|| {
            CollectorError::Adapter(String::from("missing [rust.complexity] policy"))
        })?;
    let cyclomatic = config.fn_cyclomatic.ok_or_else(|| {
        CollectorError::Adapter(String::from("missing rust.complexity.fn_cyclomatic"))
    })?;

    let target = resolve_analysis_target(context)?;
    let metrics = run_rust_code_analysis(context, &target)?;

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
            count_level(level, &mut warn_count, &mut fail_count);
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
        language: Language::Rust,
        scope: context.scope.clone(),
        pass: fail_count == 0,
        result: SignalResult::Complexity(ComplexityResult {
            engine: String::from("rust-code-analysis-cli"),
            method: String::from("ast_metrics"),
            measured_functions,
            max_fn_cyclomatic: round2(max_fn_cyclomatic),
            max_fn_cognitive: max_fn_cognitive.map(round2),
            warn_count,
            fail_count,
            failure: None,
        }),
        budget: Budget::Complexity(budget),
        offenders: Offenders::Complexity(offenders),
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

fn resolve_analysis_target(context: &RunContext) -> Result<PathBuf, CollectorError> {
    let target = if let Some(file) = &context.scope.file {
        resolve_repo_path(&context.repo_root, file)
    } else if let Some(package) = &context.scope.package {
        resolve_package_path(context, package)?
    } else if let Some(path) = &context.scope.path {
        resolve_repo_path(&context.repo_root, path)
    } else {
        context.workdir.clone()
    };
    target.canonicalize().map_err(|error| {
        CollectorError::Adapter(format!(
            "complexity scope {} could not be resolved: {error}",
            target.display()
        ))
    })
}

#[cfg(test)]
fn resolve_analysis_target_with<F>(
    context: &RunContext,
    resolve_package: F,
) -> Result<PathBuf, String>
where
    F: FnOnce(&Path, &str) -> Result<PathBuf, String>,
{
    let target = if let Some(file) = &context.scope.file {
        resolve_repo_path(&context.repo_root, file)
    } else if let Some(package) = &context.scope.package {
        resolve_package(&context.workdir, package)?
    } else if let Some(path) = &context.scope.path {
        resolve_repo_path(&context.repo_root, path)
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

fn resolve_package_path(context: &RunContext, package: &str) -> Result<PathBuf, CollectorError> {
    let metadata = load_metadata(context)?;
    metadata
        .packages
        .into_iter()
        .find(|candidate| candidate.name == package)
        .and_then(|candidate| {
            PathBuf::from(candidate.manifest_path)
                .parent()
                .map(Path::to_path_buf)
        })
        .ok_or_else(|| {
            CollectorError::Adapter(format!(
                "package scope '{package}' was not found in cargo metadata"
            ))
        })
}

fn load_metadata(context: &RunContext) -> Result<CargoMetadata, CollectorError> {
    let args = vec![
        String::from("metadata"),
        String::from("--format-version"),
        String::from("1"),
        String::from("--no-deps"),
    ];
    let output = run_command_for_context_structured(context, "cargo", &args)?;
    if !output.status.success() {
        return Err(CollectorError::Adapter(format!(
            "cargo metadata failed (exit {}): {}",
            output.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    serde_json::from_slice(&output.stdout).map_err(|error| {
        CollectorError::Adapter(format!("failed to parse cargo metadata output: {error}"))
    })
}

fn run_rust_code_analysis(
    context: &RunContext,
    target: &Path,
) -> Result<Vec<FunctionMetric>, CollectorError> {
    let canonical_repo_root = context
        .repo_root
        .canonicalize()
        .unwrap_or_else(|_| context.repo_root.to_path_buf());
    if is_adapter_known_output_target(&canonical_repo_root, target) {
        return Ok(Vec::new());
    }

    let args = rust_code_analysis_args(target);

    let output = run_command_for_context_structured(context, "rust-code-analysis-cli", &args)?;
    if !output.status.success() {
        return Err(CollectorError::Adapter(format!(
            "rust-code-analysis-cli failed (exit {}): {}",
            output.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    parse_rust_code_analysis_output(
        &String::from_utf8_lossy(&output.stdout),
        &canonical_repo_root,
        &context.execution.exec_cwd,
    )
    .map_err(CollectorError::Adapter)
}

fn rust_code_analysis_args(target: &Path) -> Vec<String> {
    let mut args = vec![
        String::from("--metrics"),
        String::from("--paths"),
        target.to_string_lossy().into_owned(),
        String::from("--language-type"),
        String::from("rust"),
        String::from("--output-format"),
        String::from("json"),
    ];
    if target.is_dir() {
        args.push(String::from("--include"));
        args.push(String::from("*.rs"));
        for directory in ADAPTER_KNOWN_OUTPUT_DIRECTORIES {
            args.push(String::from("--exclude"));
            args.push(format!("**/{directory}/**"));
        }
    }
    args
}

const ADAPTER_KNOWN_OUTPUT_DIRECTORIES: [&str; 3] = ["target", ".git", ".ayni"];

fn parse_rust_code_analysis_output(
    stdout: &str,
    repo_root: &Path,
    execution_cwd: &Path,
) -> Result<Vec<FunctionMetric>, String> {
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return Err(String::from(
            "rust-code-analysis-cli produced empty output; check the selected scope and tool installation",
        ));
    }

    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        let mut metrics = Vec::new();
        walk_metric_tree(&value, repo_root, execution_cwd, None, &mut metrics);
        if metrics.is_empty() && !confirms_zero_functions(&value) {
            return Err(String::from(
                "rust-code-analysis-cli output was valid JSON but did not contain function metrics",
            ));
        }
        return Ok(metrics);
    }

    let mut metrics = Vec::new();
    let mut parsed_lines = 0_u64;
    let mut every_document_confirms_zero_functions = true;
    for line in trimmed.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let value: Value = serde_json::from_str(line).map_err(|error| {
            format!("failed to parse rust-code-analysis-cli JSON line: {error}")
        })?;
        parsed_lines += 1;
        every_document_confirms_zero_functions &= confirms_zero_functions(&value);
        walk_metric_tree(&value, repo_root, execution_cwd, None, &mut metrics);
    }

    if parsed_lines == 0 {
        return Err(String::from(
            "rust-code-analysis-cli output was neither JSON nor NDJSON",
        ));
    }
    if metrics.is_empty() && !every_document_confirms_zero_functions {
        return Err(String::from(
            "rust-code-analysis-cli output did not contain function metrics",
        ));
    }
    Ok(metrics)
}

fn confirms_zero_functions(value: &Value) -> bool {
    let mut function_counts = Vec::new();
    collect_unit_function_counts(value, &mut function_counts);
    !function_counts.is_empty()
        && function_counts
            .into_iter()
            .all(|count| count.is_some_and(|count| count == 0.0))
}

fn collect_unit_function_counts(value: &Value, counts: &mut Vec<Option<f64>>) {
    match value {
        Value::Object(map) => {
            if map.get("kind").and_then(Value::as_str) == Some("unit") {
                counts.push(
                    map.get("metrics")
                        .and_then(Value::as_object)
                        .and_then(|metrics| metrics.get("nom"))
                        .and_then(Value::as_object)
                        .and_then(|nom| nom.get("functions"))
                        .and_then(Value::as_f64),
                );
            }
            if let Some(spaces) = map.get("spaces") {
                collect_unit_function_counts(spaces, counts);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_unit_function_counts(item, counts);
            }
        }
        _ => {}
    }
}

fn walk_metric_tree(
    value: &Value,
    repo_root: &Path,
    execution_cwd: &Path,
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
                    .map(|path| repo_relative_metric_path(repo_root, execution_cwd, path))
            } else {
                None
            };
            if file_from_unit
                .as_deref()
                .is_some_and(is_adapter_known_output_path)
            {
                return;
            }
            let effective_file = file_from_unit.as_deref().or(file_hint);

            if kind == Some("function")
                && let Some(metric) =
                    parse_function_metric(map, repo_root, execution_cwd, effective_file)
            {
                out.push(metric);
            }

            if let Some(spaces) = map.get("spaces").and_then(Value::as_array) {
                for child in spaces {
                    walk_metric_tree(child, repo_root, execution_cwd, effective_file, out);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                walk_metric_tree(item, repo_root, execution_cwd, file_hint, out);
            }
        }
        _ => {}
    }
}

fn parse_function_metric(
    map: &Map<String, Value>,
    repo_root: &Path,
    execution_cwd: &Path,
    file_fallback: Option<&str>,
) -> Option<FunctionMetric> {
    let metrics = map.get("metrics")?.as_object()?;
    let cyclomatic = metric_aggregate(metrics, &["cyclomatic", "cyclomatic_complexity"])?;
    let cognitive = metric_aggregate(metrics, &["cognitive", "cognitive_complexity"]);
    let file = metric_string(map, &["path", "file", "filepath"])
        .map(|path| repo_relative_metric_path(repo_root, execution_cwd, &path))
        .or_else(|| {
            file_fallback.map(|path| repo_relative_metric_path(repo_root, execution_cwd, path))
        })?;
    if is_adapter_known_output_path(&file) {
        return None;
    }
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

fn repo_relative_metric_path(repo_root: &Path, execution_cwd: &Path, path: &str) -> String {
    let normalized = path.replace('\\', "/");
    let normalized = normalized.trim_start_matches("./");
    let candidate = Path::new(normalized);
    if candidate.is_absolute() {
        return candidate.strip_prefix(repo_root).map_or_else(
            |_| normalized.trim_start_matches("./").to_string(),
            display_path,
        );
    }

    // rust-code-analysis reports relative paths from the command's working
    // directory. Some versions also emit repository-relative paths, so keep a
    // path that already starts with the execution-cwd prefix instead of
    // duplicating that prefix.
    if let Ok(cwd_relative) = execution_cwd.strip_prefix(repo_root)
        && !cwd_relative.as_os_str().is_empty()
        && candidate.starts_with(cwd_relative)
    {
        return display_path(candidate);
    }

    let from_execution_cwd = execution_cwd.join(candidate);
    from_execution_cwd.strip_prefix(repo_root).map_or_else(
        |_| normalized.trim_start_matches("./").to_string(),
        display_path,
    )
}

fn display_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn is_adapter_known_output_target(repo_root: &Path, target: &Path) -> bool {
    target.strip_prefix(repo_root).map_or_else(
        |_| is_adapter_known_output_path(&display_path(target)),
        |relative| is_adapter_known_output_path(&display_path(relative)),
    )
}

fn is_adapter_known_output_path(path: &str) -> bool {
    Path::new(path).components().any(|component| {
        let std::path::Component::Normal(name) = component else {
            return false;
        };
        ADAPTER_KNOWN_OUTPUT_DIRECTORIES
            .iter()
            .any(|directory| name == std::ffi::OsStr::new(directory))
    })
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
    classify_maximum(value, warn, fail)
}

fn count_level(level: Level, warn_count: &mut u64, fail_count: &mut u64) {
    match level {
        Level::Warn => *warn_count += 1,
        Level::Fail => *fail_count += 1,
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
    use super::{
        count_level, is_adapter_known_output_target, parse_function_metric,
        parse_rust_code_analysis_output, resolve_analysis_target_with, rust_code_analysis_args,
        threshold_level,
    };
    use ayni_core::{AyniPolicy, ExecutionResolution, Level, RunContext, Scope};
    use serde_json::json;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn context(root: &std::path::Path, scope: Scope) -> RunContext {
        RunContext {
            repo_root: root.to_path_buf(),
            target_root: root.to_path_buf(),
            workdir: root.to_path_buf(),
            policy: AyniPolicy::default(),
            scope,
            execution: ExecutionResolution::direct("cargo", root.to_path_buf(), "test", 100),
            cancellation: Default::default(),
            debug: false,
        }
    }

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
    fn accepts_valid_zero_function_evidence_but_rejects_unrecognized_json() {
        let temp = TempDir::new().expect("tempdir");
        let root = temp.path();
        let zero_functions = json!({
            "kind": "unit",
            "name": format!("{}/core/src/lib.rs", root.display()),
            "spaces": [],
            "metrics": { "nom": { "functions": 0.0 } }
        });

        let metrics = parse_rust_code_analysis_output(&zero_functions.to_string(), root, root)
            .expect("zero functions are complete evidence");
        assert!(metrics.is_empty());

        let error = parse_rust_code_analysis_output(
            &json!({"kind": "unit", "spaces": [], "metrics": {}}).to_string(),
            root,
            root,
        )
        .expect_err("unrecognized output must fail closed");
        assert!(error.contains("did not contain function metrics"));
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

    #[test]
    fn normalizes_paths_from_workspace_execution_cwd_for_a_nested_configured_root() {
        let temp = TempDir::new().expect("tempdir");
        let root = temp.path();
        let execution_cwd = root;
        let payload = json!([
            {
                "kind": "unit",
                "name": "src/relative_to_execution_cwd.rs",
                "spaces": [{
                    "kind": "function",
                    "name": "relative_to_execution_cwd",
                    "start_line": 4,
                    "metrics": { "cyclomatic": { "max": 3.0 } }
                }]
            },
            {
                "kind": "unit",
                "name": "crates/api/src/already_repo_relative.rs",
                "spaces": [{
                    "kind": "function",
                    "name": "already_repo_relative",
                    "start_line": 8,
                    "metrics": { "cyclomatic": { "max": 4.0 } }
                }]
            },
            {
                "kind": "unit",
                "name": "./crates/api/src/dot_repo_relative.rs",
                "spaces": [{
                    "kind": "function",
                    "name": "dot_repo_relative",
                    "start_line": 12,
                    "metrics": { "cyclomatic": { "max": 5.0 } }
                }]
            }
        ]);

        let metrics = parse_rust_code_analysis_output(&payload.to_string(), root, execution_cwd)
            .expect("metrics");

        assert_eq!(metrics.len(), 3);
        assert_eq!(metrics[0].file, "src/relative_to_execution_cwd.rs");
        assert_eq!(metrics[1].file, "crates/api/src/already_repo_relative.rs");
        assert_eq!(metrics[2].file, "crates/api/src/dot_repo_relative.rs");
    }

    #[test]
    fn skips_adapter_known_generated_output_paths() {
        let temp = TempDir::new().expect("tempdir");
        let root = temp.path();
        let payload = json!([
            {
                "kind": "unit",
                "name": "src/kept.rs",
                "spaces": [{
                    "kind": "function",
                    "name": "kept",
                    "metrics": { "cyclomatic": { "max": 3.0 } }
                }]
            },
            {
                "kind": "unit",
                "name": "target/generated.rs",
                "spaces": [{
                    "kind": "function",
                    "name": "generated",
                    "metrics": { "cyclomatic": { "max": 30.0 } }
                }]
            },
            {
                "kind": "unit",
                "name": ".git/hooks/ignored.rs",
                "spaces": [{
                    "kind": "function",
                    "name": "git_hook",
                    "metrics": { "cyclomatic": { "max": 30.0 } }
                }]
            },
            {
                "kind": "unit",
                "name": ".ayni/cache/ignored.rs",
                "spaces": [{
                    "kind": "function",
                    "name": "artifact",
                    "metrics": { "cyclomatic": { "max": 30.0 } }
                }]
            }
        ]);

        let metrics =
            parse_rust_code_analysis_output(&payload.to_string(), root, root).expect("metrics");

        assert_eq!(metrics.len(), 1);
        assert_eq!(metrics[0].file, "src/kept.rs");
    }

    #[test]
    fn does_not_analyze_an_adapter_known_output_target() {
        let temp = TempDir::new().expect("tempdir");
        let root = temp.path();

        assert!(is_adapter_known_output_target(
            root,
            &root.join("target/generated.rs")
        ));
        assert!(!is_adapter_known_output_target(
            root,
            &root.join("src/lib.rs")
        ));
    }

    #[test]
    fn excludes_adapter_known_outputs_from_directory_analysis() {
        let temp = TempDir::new().expect("tempdir");
        let args = rust_code_analysis_args(temp.path());

        for pattern in ["**/target/**", "**/.git/**", "**/.ayni/**"] {
            assert!(
                args.windows(2)
                    .any(|pair| { pair[0] == "--exclude" && pair[1] == pattern })
            );
        }
    }

    #[test]
    fn file_scope_resolves_the_exact_file() {
        let temp = TempDir::new().expect("tempdir");
        fs::create_dir_all(temp.path().join("src")).expect("src");
        fs::write(temp.path().join("src/lib.rs"), "fn example() {}\n").expect("file");
        let context = context(
            temp.path(),
            Scope {
                file: Some(String::from("src/lib.rs")),
                ..Scope::default()
            },
        );

        let target = resolve_analysis_target_with(&context, |_, _| {
            Err(String::from("package resolver must not be called"))
        })
        .expect("file target");

        assert_eq!(
            target,
            temp.path()
                .join("src/lib.rs")
                .canonicalize()
                .expect("canonical")
        );
    }

    #[test]
    fn package_scope_takes_precedence_over_the_broader_path_scope() {
        let temp = TempDir::new().expect("tempdir");
        fs::create_dir_all(temp.path().join("crates/api")).expect("package");
        let context = context(
            temp.path(),
            Scope {
                path: Some(String::from(".")),
                package: Some(String::from("api")),
                ..Scope::default()
            },
        );

        let target = resolve_analysis_target_with(&context, |workdir, package| {
            assert_eq!(workdir, temp.path());
            assert_eq!(package, "api");
            Ok(PathBuf::from(workdir).join("crates/api"))
        })
        .expect("package target");

        assert_eq!(
            target,
            temp.path()
                .join("crates/api")
                .canonicalize()
                .expect("canonical")
        );
    }

    #[test]
    fn maximum_threshold_equality() {
        let mut warn_count = 0;
        let mut fail_count = 0;
        assert_eq!(threshold_level(9.0, 10.0, 15.0), None);
        let warn = threshold_level(10.0, 10.0, 15.0).expect("warn offender");
        count_level(warn, &mut warn_count, &mut fail_count);
        assert_eq!(warn, Level::Warn);
        assert_eq!(threshold_level(14.0, 10.0, 15.0), Some(Level::Warn));
        let fail = threshold_level(15.0, 10.0, 15.0).expect("fail offender");
        count_level(fail, &mut warn_count, &mut fail_count);
        assert_eq!(fail, Level::Fail);
        assert_eq!((warn_count, fail_count), (1, 1));
    }
}
