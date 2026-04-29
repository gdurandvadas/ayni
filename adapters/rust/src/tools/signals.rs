use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use ayni_core::AyniPolicy;
use glob::Pattern;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use walkdir::WalkDir;

mod helpers;

use helpers::{round2, sort_offenders_desc_numeric, to_relative_posix};

const CHECK_SIZE: &str = "size";
const CHECK_COMPLEXITY: &str = "complexity";
const CHECK_DEPS: &str = "deps";
const CHECK_MUTATION: &str = "mutation";

const ENGINE_COMPLEXITY: &str = "rust-code-analysis-cli";
const METHOD_COMPLEXITY: &str = "ast_metrics";

const IGNORE_PARTS: [&str; 4] = [".git", "target", "node_modules", "build"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalCheck {
    Size,
    Complexity,
    Deps,
    Mutation,
}

impl SignalCheck {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Size => CHECK_SIZE,
            Self::Complexity => CHECK_COMPLEXITY,
            Self::Deps => CHECK_DEPS,
            Self::Mutation => CHECK_MUTATION,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct SignalScopeInput {
    pub scope: Option<String>,
    pub file: Option<String>,
    /// Language-agnostic package/module identifier. For Rust: resolved first as a
    /// crate name, then as a package path. Other adapters interpret it accordingly.
    pub package: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckResult {
    pub check: String,
    pub scope: Value,
    pub pass: bool,
    pub result: Value,
    pub budget: Value,
    pub delta_vs_baseline: Value,
    pub delta_vs_previous: Value,
    pub offenders: Vec<Value>,
}

#[derive(Debug, Clone)]
struct ScopeContext {
    repo_root: PathBuf,
    root_relative: String,
    root_absolute: PathBuf,
    file: Option<String>,
    package: Option<String>,
    scope: Option<String>,
}

#[derive(Debug, Clone)]
struct CrateInfo {
    name: String,
    path: String,
    cargo_path: PathBuf,
    dependencies: BTreeMap<String, toml::Value>,
}

#[derive(Debug, Clone)]
struct Edge {
    from: String,
    to: String,
}

pub fn run_signal_checks(
    repo_root: &Path,
    checks: &[SignalCheck],
    scope_input: &SignalScopeInput,
) -> Result<Vec<CheckResult>, String> {
    let policy = AyniPolicy::load(repo_root)?;
    let crates = workspace_crates(repo_root)?;
    let scope = resolve_scope(repo_root, scope_input, &crates)?;

    checks
        .iter()
        .copied()
        .map(|check| run_single_check(&scope, &policy, &crates, check))
        .collect()
}

fn run_single_check(
    scope: &ScopeContext,
    policy: &AyniPolicy,
    crates: &[CrateInfo],
    check: SignalCheck,
) -> Result<CheckResult, String> {
    match check {
        SignalCheck::Size => run_size_check(scope, policy),
        SignalCheck::Complexity => run_complexity_check(scope, policy),
        SignalCheck::Deps => run_deps_check(scope, policy, crates),
        SignalCheck::Mutation => run_mutation_check(scope),
    }
}

fn run_size_check(scope: &ScopeContext, policy: &AyniPolicy) -> Result<CheckResult, String> {
    let rs_budget = policy
        .size
        .thresholds
        .get("*.rs")
        .ok_or_else(|| String::from("missing size.thresholds['*.rs'] policy"))?;

    let files = collect_rs_files(scope)?;
    let mut offenders: Vec<Value> = Vec::new();
    let mut max_lines = 0_u64;
    let mut warn_count = 0_u64;
    let mut fail_count = 0_u64;

    for relative in &files {
        let absolute = scope.repo_root.join(relative);
        let content = fs::read_to_string(&absolute)
            .map_err(|err| format!("failed to read {}: {err}", absolute.display()))?;
        let line_count = content.lines().count() as u64;
        max_lines = max_lines.max(line_count);

        if line_count > rs_budget.warn {
            let level = if line_count > rs_budget.fail {
                fail_count += 1;
                "fail"
            } else {
                warn_count += 1;
                "warn"
            };

            offenders.push(json!({
                "file": relative,
                "value": line_count,
                "warn": rs_budget.warn,
                "fail": rs_budget.fail,
                "level": level,
            }));
        }
    }

    sort_offenders_desc_numeric(&mut offenders);

    let result = json!({
        "max_lines": max_lines,
        "total_files": files.len() as u64,
        "warn_count": warn_count,
        "fail_count": fail_count,
    });
    let budget = json!({
        "rs": {
            "warn": rs_budget.warn,
            "fail": rs_budget.fail,
        }
    });
    Ok(CheckResult {
        check: String::from(CHECK_SIZE),
        scope: scope_to_json(scope),
        pass: fail_count == 0,
        result,
        budget,
        delta_vs_baseline: json!({}),
        delta_vs_previous: json!({}),
        offenders,
    })
}

fn run_complexity_check(scope: &ScopeContext, policy: &AyniPolicy) -> Result<CheckResult, String> {
    let cyclomatic = policy
        .complexity
        .fn_cyclomatic
        .ok_or_else(|| String::from("missing complexity.fn_cyclomatic policy"))?;

    let files = collect_rs_files(scope)?;
    let metrics = run_rust_code_analysis(scope)?;

    let mut offenders: Vec<Value> = Vec::new();
    let mut max_cyclomatic = 0.0_f64;
    let mut max_cognitive = 0.0_f64;
    let mut warn_count = 0_u64;
    let mut fail_count = 0_u64;
    let mut measured_functions = 0_u64;

    for metric in &metrics {
        measured_functions += 1;
        max_cyclomatic = max_cyclomatic.max(metric.cyclomatic);
        max_cognitive = max_cognitive.max(metric.cognitive);

        let cyclomatic_level = threshold_level(metric.cyclomatic, cyclomatic.warn, cyclomatic.fail);
        let cognitive_level = policy
            .complexity
            .fn_cognitive
            .map(|threshold| threshold_level(metric.cognitive, threshold.warn, threshold.fail))
            .unwrap_or(None);
        let level = max_level(cyclomatic_level, cognitive_level);

        if let Some(level) = level {
            if level == "fail" {
                fail_count += 1;
            } else {
                warn_count += 1;
            }
            offenders.push(json!({
                "file": metric.file,
                "line": metric.line,
                "function": metric.function,
                "value": round2(metric.cyclomatic),
                "cyclomatic": round2(metric.cyclomatic),
                "cognitive": round2(metric.cognitive),
                "level": level,
            }));
        }
    }

    sort_offenders_desc_numeric(&mut offenders);

    let result = json!({
        "engine": ENGINE_COMPLEXITY,
        "method": METHOD_COMPLEXITY,
        "file_count": files.len() as u64,
        "measured_functions": measured_functions,
        "max_fn_cyclomatic": round2(max_cyclomatic),
        "max_fn_cognitive": round2(max_cognitive),
        "warn_count": warn_count,
        "fail_count": fail_count,
    });
    let mut budget = json!({
        "fn_cyclomatic": {
            "warn": cyclomatic.warn,
            "fail": cyclomatic.fail,
        }
    });
    if let Some(cognitive) = policy.complexity.fn_cognitive
        && let Some(map) = budget.as_object_mut()
    {
        map.insert(
            String::from("fn_cognitive"),
            json!({"warn": cognitive.warn, "fail": cognitive.fail}),
        );
    }
    Ok(CheckResult {
        check: String::from(CHECK_COMPLEXITY),
        scope: scope_to_json(scope),
        pass: fail_count == 0,
        result,
        budget,
        delta_vs_baseline: json!({}),
        delta_vs_previous: json!({}),
        offenders,
    })
}

#[derive(Debug, Clone)]
struct FunctionMetric {
    file: String,
    line: u64,
    function: String,
    cyclomatic: f64,
    cognitive: f64,
}

fn run_rust_code_analysis(scope: &ScopeContext) -> Result<Vec<FunctionMetric>, String> {
    let target = scope.root_absolute.canonicalize().map_err(|err| {
        format!(
            "complexity scope {} could not be resolved: {err}",
            scope.root_absolute.display()
        )
    })?;

    let mut command = Command::new("rust-code-analysis-cli");
    command
        .arg("--metrics")
        .arg("--paths")
        .arg(&target)
        .arg("--language-type")
        .arg("rust")
        .arg("--output-format")
        .arg("json")
        .current_dir(&scope.repo_root);
    if target.is_dir() {
        command.arg("--include").arg("*.rs");
    }

    let output = command
        .output()
        .map_err(|err| format!("failed to execute rust-code-analysis-cli: {err}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "rust-code-analysis-cli failed (exit {}): {}",
            output.status.code().unwrap_or(-1),
            stderr.trim()
        ));
    }

    if stdout.trim().is_empty() {
        return Err(String::from(
            "rust-code-analysis-cli produced empty output; use a canonical path (not bare `.`) or file path",
        ));
    }

    let mut metrics = Vec::new();
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let parsed: Value = serde_json::from_str(line)
            .map_err(|err| format!("failed to parse rust-code-analysis-cli JSON line: {err}"))?;
        walk_metric_tree(&parsed, scope.repo_root.as_path(), None, &mut metrics);
    }
    Ok(metrics)
}

/// Walk Mozilla rust-code-analysis JSON: one `unit` per file (NDJSON lines), nested `spaces`
/// with `function` leaves. Produces per-function metrics matching `.ayni.toml` thresholds.
fn walk_metric_tree(
    value: &Value,
    repo_root: &Path,
    file_hint: Option<&str>,
    out: &mut Vec<FunctionMetric>,
) {
    match value {
        Value::Object(map) => {
            let kind = map.get("kind").and_then(Value::as_str);
            let file_from_unit = if kind == Some("unit") {
                map.get("name")
                    .and_then(Value::as_str)
                    .filter(|n| n.contains('/') || n.ends_with(".rs"))
                    .map(|p| repo_relative_metric_path(repo_root, p))
            } else {
                None
            };
            let effective = file_from_unit.as_deref().or(file_hint);

            if kind == Some("function")
                && let Some(metric) = parse_function_metric(map, repo_root, effective)
            {
                out.push(metric);
            }

            if let Some(spaces) = map.get("spaces").and_then(Value::as_array) {
                let next = effective.or(file_hint);
                for child in spaces {
                    walk_metric_tree(child, repo_root, next, out);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                walk_metric_tree(item, repo_root, file_hint, out);
            }
        }
        _ => {}
    }
}

fn parse_function_metric(
    map: &Map<String, Value>,
    repo_root: &Path,
    file_fallback: Option<&str>,
) -> Option<FunctionMetric> {
    let metrics = map.get("metrics")?.as_object()?;
    let cyclomatic = metric_aggregate(metrics, &["cyclomatic", "cyclomatic_complexity"])?;
    let cognitive =
        metric_aggregate(metrics, &["cognitive", "cognitive_complexity"]).unwrap_or(0.0);

    let file = metric_string(map, &["path", "file", "filepath"])
        .map(|p| repo_relative_metric_path(repo_root, &p))
        .or_else(|| file_fallback.map(|p| repo_relative_metric_path(repo_root, p)))?;

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

fn repo_relative_metric_path(repo_root: &Path, path: &str) -> String {
    let normalized = path.replace('\\', "/");
    if let Ok(root) = repo_root.canonicalize() {
        let root_s = root.to_string_lossy().replace('\\', "/");
        if let Some(rest) = normalized.strip_prefix(&root_s) {
            return rest.trim_start_matches('/').to_string();
        }
    }
    normalized.trim_start_matches("./").trim().to_string()
}

/// `rust-code-analysis` may emit a float or an object `{ sum, min, max, average }`.
fn metric_aggregate(map: &Map<String, Value>, keys: &[&str]) -> Option<f64> {
    for key in keys {
        match map.get(*key) {
            Some(Value::Number(n)) => return n.as_f64(),
            Some(Value::Object(obj)) => {
                if let Some(v) = obj.get("max").and_then(Value::as_f64) {
                    return Some(v);
                }
                if let Some(v) = obj.get("sum").and_then(Value::as_f64) {
                    return Some(v);
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

fn threshold_level(value: f64, warn: f64, fail: f64) -> Option<&'static str> {
    if value > fail {
        Some("fail")
    } else if value > warn {
        Some("warn")
    } else {
        None
    }
}

fn max_level(left: Option<&'static str>, right: Option<&'static str>) -> Option<&'static str> {
    match (left, right) {
        (Some("fail"), _) | (_, Some("fail")) => Some("fail"),
        (Some(_), _) | (_, Some(_)) => Some("warn"),
        _ => None,
    }
}

fn run_deps_check(
    scope: &ScopeContext,
    policy: &AyniPolicy,
    crates: &[CrateInfo],
) -> Result<CheckResult, String> {
    let forbidden = &policy.deps.rust.forbidden;

    let visible_crates = scoped_crates(scope, crates);
    let visible_set: HashSet<&str> = visible_crates
        .iter()
        .map(|info| info.path.as_str())
        .collect();
    let name_map: HashMap<&str, &str> = crates
        .iter()
        .map(|info| (info.name.as_str(), info.path.as_str()))
        .collect();

    let mut edges: Vec<Edge> = Vec::new();
    for info in &visible_crates {
        for (dep_name, dep_value) in &info.dependencies {
            if let Some(to_path) = resolve_dependency_path(
                scope.repo_root.as_path(),
                info.cargo_path
                    .parent()
                    .unwrap_or(scope.repo_root.as_path()),
                dep_name,
                dep_value,
                &name_map,
            ) && visible_set.contains(to_path.as_str())
            {
                edges.push(Edge {
                    from: info.path.clone(),
                    to: to_path,
                });
            }
        }
    }

    let mut offenders: Vec<Value> = Vec::new();
    for edge in &edges {
        let src_class = path_class(&edge.from);
        let dst_class = path_class(&edge.to);
        if let Some(patterns) = forbidden.get(src_class.as_str()) {
            for pattern in patterns {
                let pattern_obj = Pattern::new(pattern)
                    .map_err(|err| format!("invalid forbidden deps pattern '{pattern}': {err}"))?;
                if pattern_obj.matches(dst_class.as_str()) {
                    offenders.push(json!({
                        "file": format!("{}/Cargo.toml", edge.from),
                        "value": format!("{} -> {}", edge.from, edge.to),
                        "from": edge.from,
                        "to": edge.to,
                        "rule": src_class,
                        "forbidden": pattern,
                    }));
                }
            }
        }
    }

    offenders.sort_by(|left, right| {
        let left_value = left
            .get("value")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let right_value = right
            .get("value")
            .and_then(Value::as_str)
            .unwrap_or_default();
        right_value.cmp(left_value)
    });

    let result = json!({
        "crate_count": visible_crates.len() as u64,
        "edge_count": edges.len() as u64,
        "violation_count": offenders.len() as u64,
    });
    let budget = json!({
        "forbidden": forbidden,
    });
    Ok(CheckResult {
        check: String::from(CHECK_DEPS),
        scope: scope_to_json(scope),
        pass: offenders.is_empty(),
        result,
        budget,
        delta_vs_baseline: json!({}),
        delta_vs_previous: json!({}),
        offenders,
    })
}

fn run_mutation_check(scope: &ScopeContext) -> Result<CheckResult, String> {
    let merge_base = resolve_merge_base(scope.repo_root.as_path())?;
    let diff_path = scope.repo_root.join(".ayni/branch.diff");
    if let Some(parent) = diff_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
    }

    let diff_output = Command::new("git")
        .arg("diff")
        .arg(format!("{merge_base}...HEAD"))
        .current_dir(&scope.repo_root)
        .output()
        .map_err(|err| format!("failed to run git diff for mutation scope: {err}"))?;
    if !diff_output.status.success() {
        return Err(format!(
            "git diff failed for mutation scope (exit {}): {}",
            diff_output.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&diff_output.stderr).trim()
        ));
    }
    fs::write(&diff_path, &diff_output.stdout)
        .map_err(|err| format!("failed to write {}: {err}", diff_path.display()))?;

    let mutants_output = Command::new("cargo")
        .arg("mutants")
        .arg("--in-diff")
        .arg(diff_path.as_os_str())
        .arg("--no-times")
        .current_dir(&scope.repo_root)
        .output()
        .map_err(|err| format!("failed to execute cargo mutants: {err}"))?;

    let missed = read_mutants_lines(scope.repo_root.as_path(), "missed.txt")?;
    let timeout = read_mutants_lines(scope.repo_root.as_path(), "timeout.txt")?;
    let caught = read_mutants_lines(scope.repo_root.as_path(), "caught.txt")?;
    let total = (missed.len() + timeout.len() + caught.len()) as u64;
    let missed_count = missed.len() as u64;
    let timeout_count = timeout.len() as u64;
    let caught_count = caught.len() as u64;
    let score = if total == 0 {
        0.0
    } else {
        round2((caught_count as f64 / total as f64) * 100.0)
    };

    let mut offenders = Vec::new();
    for line in missed {
        offenders.push(mutant_offender(&line, "fail"));
    }
    for line in timeout {
        offenders.push(mutant_offender(&line, "warn"));
    }

    let result = json!({
        "engine": "cargo-mutants",
        "mode": "in-diff",
        "merge_base": merge_base,
        "diff_file": ".ayni/branch.diff",
        "total_mutants": total,
        "caught_count": caught_count,
        "missed_count": missed_count,
        "timeout_count": timeout_count,
        "mutation_score": score,
        "exit_code": mutants_output.status.code(),
    });
    let budget = json!({});
    Ok(CheckResult {
        check: String::from(CHECK_MUTATION),
        scope: scope_to_json(scope),
        pass: missed_count == 0 && timeout_count == 0,
        result,
        budget,
        delta_vs_baseline: json!({}),
        delta_vs_previous: json!({}),
        offenders,
    })
}

fn resolve_merge_base(repo_root: &Path) -> Result<String, String> {
    if let Ok(value) = std::env::var("AYNI_MERGE_BASE") {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return Ok(String::from(trimmed));
        }
    }

    for candidate in ["origin/main", "main"] {
        let status = Command::new("git")
            .arg("rev-parse")
            .arg("--verify")
            .arg(candidate)
            .current_dir(repo_root)
            .status()
            .map_err(|err| format!("failed to resolve merge base candidate {candidate}: {err}"))?;
        if status.success() {
            return Ok(String::from(candidate));
        }
    }

    Ok(String::from("HEAD~1"))
}

fn read_mutants_lines(repo_root: &Path, file_name: &str) -> Result<Vec<String>, String> {
    let path = repo_root.join("mutants.out").join(file_name);
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let content = fs::read_to_string(&path)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    Ok(content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(String::from)
        .collect())
}

fn mutant_offender(line: &str, level: &str) -> Value {
    let mut offender = json!({
        "value": line,
        "level": level,
    });
    if let Some((file, line_no)) = parse_path_line_prefix(line)
        && let Some(map) = offender.as_object_mut()
    {
        map.insert(String::from("file"), Value::String(file));
        map.insert(String::from("line"), Value::Number(line_no.into()));
    }
    offender
}

fn parse_path_line_prefix(value: &str) -> Option<(String, u64)> {
    let mut parts = value.splitn(3, ':');
    let file = parts.next()?.trim();
    let line_text = parts.next()?.trim();
    let line = line_text.parse::<u64>().ok()?;
    if file.is_empty() {
        return None;
    }
    Some((String::from(file), line))
}

fn resolve_scope(
    repo_root: &Path,
    input: &SignalScopeInput,
    crates: &[CrateInfo],
) -> Result<ScopeContext, String> {
    let root_relative = if let Some(file) = &input.file {
        file.clone()
    } else if let Some(scope) = &input.scope {
        scope.clone()
    } else if let Some(package) = &input.package {
        // Try resolving as a crate name first, then fall back to treating it as a path.
        resolve_crate_path(package, crates)?.unwrap_or_else(|| package.clone())
    } else {
        String::from(".")
    };

    let absolute = if Path::new(&root_relative).is_absolute() {
        PathBuf::from(&root_relative)
    } else {
        repo_root.join(&root_relative)
    };

    Ok(ScopeContext {
        repo_root: repo_root.to_path_buf(),
        root_relative,
        root_absolute: absolute,
        file: input.file.clone(),
        package: input.package.clone(),
        scope: input.scope.clone(),
    })
}

fn scope_to_json(scope: &ScopeContext) -> Value {
    let mut map = Map::new();
    map.insert(
        String::from("root"),
        Value::String(scope.root_relative.clone()),
    );
    if let Some(file) = &scope.file {
        map.insert(String::from("file"), Value::String(file.clone()));
    }
    if let Some(package) = &scope.package {
        map.insert(String::from("package"), Value::String(package.clone()));
    }
    if let Some(scope_value) = &scope.scope {
        map.insert(String::from("scope"), Value::String(scope_value.clone()));
    }
    Value::Object(map)
}

fn collect_rs_files(scope: &ScopeContext) -> Result<Vec<String>, String> {
    if !scope.root_absolute.exists() {
        return Ok(Vec::new());
    }

    if scope.root_absolute.is_file() {
        if scope
            .root_absolute
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext == "rs")
        {
            return to_relative_posix(scope.repo_root.as_path(), &scope.root_absolute)
                .map(|relative| vec![relative]);
        }
        return Ok(Vec::new());
    }

    let mut files: Vec<String> = WalkDir::new(&scope.root_absolute)
        .into_iter()
        .filter_entry(|entry| !contains_ignored_part(entry.path()))
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter_map(|entry| {
            let path = entry.into_path();
            if path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext == "rs")
            {
                to_relative_posix(scope.repo_root.as_path(), path.as_path()).ok()
            } else {
                None
            }
        })
        .collect();
    files.sort();
    files.dedup();
    Ok(files)
}

fn contains_ignored_part(path: &Path) -> bool {
    path.components().any(|component| {
        let value = component.as_os_str().to_string_lossy();
        IGNORE_PARTS.iter().any(|ignore| *ignore == value)
    })
}

fn workspace_crates(repo_root: &Path) -> Result<Vec<CrateInfo>, String> {
    let mut crates: Vec<CrateInfo> = Vec::new();

    for entry in WalkDir::new(repo_root)
        .into_iter()
        .filter_entry(|entry| !contains_ignored_part(entry.path()))
        .filter_map(Result::ok)
    {
        let path = entry.path();
        if !entry.file_type().is_file()
            || path.file_name().and_then(|name| name.to_str()) != Some("Cargo.toml")
        {
            continue;
        }
        if path.parent().is_some_and(|parent| parent == repo_root) {
            continue;
        }

        let content = fs::read_to_string(path)
            .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
        let value = toml::from_str::<toml::Value>(&content)
            .map_err(|err| format!("failed to parse {}: {err}", path.display()))?;
        let package = value.get("package").and_then(toml::Value::as_table);
        let Some(package_table) = package else {
            continue;
        };

        let Some(name) = package_table
            .get("name")
            .and_then(toml::Value::as_str)
            .map(String::from)
        else {
            continue;
        };

        let dependencies = value
            .get("dependencies")
            .and_then(toml::Value::as_table)
            .map(|table| {
                table
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect::<BTreeMap<_, _>>()
            })
            .unwrap_or_default();

        let parent = path
            .parent()
            .ok_or_else(|| format!("invalid Cargo.toml path: {}", path.display()))?;
        let relative = to_relative_posix(repo_root, parent)?;
        crates.push(CrateInfo {
            name,
            path: relative,
            cargo_path: path.to_path_buf(),
            dependencies,
        });
    }

    crates.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(crates)
}

fn resolve_crate_path(crate_name: &str, crates: &[CrateInfo]) -> Result<Option<String>, String> {
    let mut matches = crates
        .iter()
        .filter(|info| info.name == crate_name)
        .map(|info| info.path.clone());
    let first = matches.next();
    if matches.next().is_some() {
        return Err(format!("multiple crates found for '{crate_name}'"));
    }
    Ok(first)
}

fn scoped_crates<'a>(scope: &ScopeContext, crates: &'a [CrateInfo]) -> Vec<&'a CrateInfo> {
    if scope.root_relative == "." {
        return crates.iter().collect();
    }

    let root = scope.root_relative.trim_end_matches('/');
    crates
        .iter()
        .filter(|info| {
            info.path == root
                || info.path.starts_with(&format!("{root}/"))
                || root.starts_with(&format!("{}/", info.path))
        })
        .collect()
}

fn resolve_dependency_path(
    repo_root: &Path,
    crate_dir: &Path,
    dep_name: &str,
    dep_value: &toml::Value,
    crate_name_map: &HashMap<&str, &str>,
) -> Option<String> {
    if let Some(path) = crate_name_map.get(dep_name) {
        return Some(String::from(*path));
    }

    let table = dep_value.as_table()?;
    let dep_path = table.get("path")?.as_str()?;
    let absolute = crate_dir.join(dep_path).canonicalize().ok()?;
    to_relative_posix(repo_root, &absolute).ok()
}

fn path_class(path: &str) -> String {
    if path == "core" {
        return String::from("core");
    }
    if path.starts_with("adapters/") {
        return String::from("adapters/*");
    }
    if path == "cli" {
        return String::from("cli");
    }
    String::from(path)
}

#[cfg(test)]
mod tests;
