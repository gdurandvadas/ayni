use ayni_core::{
    Budget, CoverageOffender, CoveragePolicy, CoverageResult, Level, Offenders, RunContext, Scope,
    SignalKind, SignalResult, SignalRow,
};
use serde_json::{Value as JsonValue, json};
use std::process::Command;

pub fn collect(context: &RunContext) -> Result<SignalRow, String> {
    let (program, args, engine_label) = coverage_command(context);
    let command_text = format_command(&program, &args);
    let output = Command::new(&program)
        .args(args.iter().map(String::as_str))
        .current_dir(&context.workdir)
        .output()
        .map_err(|error| format!("failed to execute {command_text}: {error}"))?;

    let (status, percent, line_percent, branch_percent) = if output.status.success() {
        let payload: JsonValue = serde_json::from_slice(&output.stdout)
            .map_err(|error| format!("failed to parse cargo llvm-cov output: {error}"))?;
        let (line, branch) = find_coverage_percents(&payload);
        let percent = line.or(branch);
        (String::from("ok"), percent, line, branch)
    } else {
        (String::from("error"), None, None, None)
    };

    let coverage_config = context.policy.rust.coverage.as_ref();
    let coverage_budget = coverage_config
        .map(|config| {
            json!({
                "line_percent_warn": config.line_percent.map(|v| v.warn),
                "line_percent_fail": config.line_percent.map(|v| v.fail),
            })
        })
        .unwrap_or_else(|| json!({}));

    // Evaluate pass: tool must succeed AND measured percent must meet fail threshold.
    let headline = percent.or(line_percent).or(branch_percent);
    let pass = status == "ok"
        && coverage_config
            .and_then(|c| c.line_percent)
            .is_none_or(|t| headline.is_none_or(|v| v >= t.fail));

    let offenders = build_offenders(headline, coverage_config);

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
        }),
        budget: Budget::Coverage(coverage_budget),
        offenders: Offenders::Coverage(offenders),
        delta_vs_previous: None,
        delta_vs_baseline: None,
    })
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

fn format_command(program: &str, args: &[String]) -> String {
    if args.is_empty() {
        program.to_string()
    } else {
        format!("{program} {}", args.join(" "))
    }
}

fn build_offenders(
    headline: Option<f64>,
    policy: Option<&CoveragePolicy>,
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
        file: String::from("<workspace>"),
        line: None,
        value,
        level,
    }]
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
        .and_then(JsonValue::as_f64)
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
        if let Some(number) = map.get(*key).and_then(JsonValue::as_f64) {
            return Some(number);
        }
        if let Some(obj) = map.get(*key).and_then(JsonValue::as_object) {
            for nested in nested_keys {
                if let Some(number) = obj.get(*nested).and_then(JsonValue::as_f64) {
                    return Some(number);
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{build_offenders, coverage_command, find_coverage_percents};
    use ayni_core::{AyniPolicy, CoveragePolicy, Level, RunContext, Scope, ThresholdFloat};
    use serde_json::json;
    use std::path::PathBuf;

    fn context_with_policy(document: &str) -> RunContext {
        let policy: AyniPolicy = toml::from_str(document).expect("policy");
        RunContext {
            repo_root: PathBuf::from("."),
            workdir: PathBuf::from("."),
            policy,
            scope: Scope::default(),
            diff: None,
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
    fn pass_when_above_warn() {
        assert!(build_offenders(Some(80.0), Some(&policy(70.0, 50.0))).is_empty());
    }

    #[test]
    fn warn_offender_when_between_warn_and_fail() {
        let offenders = build_offenders(Some(60.0), Some(&policy(70.0, 50.0)));
        assert_eq!(offenders.len(), 1);
        assert_eq!(offenders[0].level, Level::Warn);
    }

    #[test]
    fn fail_offender_when_below_fail() {
        let offenders = build_offenders(Some(28.9), Some(&policy(70.0, 50.0)));
        assert_eq!(offenders.len(), 1);
        assert_eq!(offenders[0].level, Level::Fail);
        assert!((offenders[0].value - 28.9).abs() < 0.01);
    }

    #[test]
    fn pass_when_no_policy() {
        assert!(build_offenders(Some(1.0), None).is_empty());
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
