use std::collections::BTreeMap;

use ayni_core::{Level, Offenders, RunArtifact, SignalResult, SignalRow};
use serde_json::Value;

const PASS_IMAGE_URL: &str =
    "https://raw.githubusercontent.com/gdurandvadas/ayni/refs/heads/main/assets/pass.svg";
const WARN_IMAGE_URL: &str =
    "https://raw.githubusercontent.com/gdurandvadas/ayni/refs/heads/main/assets/warn.svg";
const FAIL_IMAGE_URL: &str =
    "https://raw.githubusercontent.com/gdurandvadas/ayni/refs/heads/main/assets/fail.svg";

pub fn build_markdown(artifact: &RunArtifact, offenders_limit: usize) -> String {
    let mut out = String::new();
    let total = artifact.rows.len();
    let passing = artifact.rows.iter().filter(|row| row.pass).count();
    out.push_str("# ayni analyze\n\n");
    out.push_str(&format!(
        "**{}** / **{}** checks passing · schema `{}`\n\n",
        passing, total, artifact.schema_version
    ));

    let mut grouped = BTreeMap::<String, Vec<&SignalRow>>::new();
    for row in &artifact.rows {
        let root = row.scope.path.as_deref().unwrap_or(".");
        grouped
            .entry(format!("{}:{}", row.language.as_str(), root))
            .or_default()
            .push(row);
    }

    for (group, rows) in grouped {
        let mut parts = group.splitn(2, ':');
        let language = parts.next().unwrap_or("unknown");
        let root = parts.next().unwrap_or(".");
        let root_label = if root == "." { "workspace" } else { root };
        let group_pass = rows.iter().filter(|row| row.pass).count();
        out.push_str(&format!(
            "## {} ({}) — {}/{} passing\n\n",
            language,
            root_label,
            group_pass,
            rows.len()
        ));

        out.push_str("| # | Status | Signal | Summary | Delta |\n");
        out.push_str("|---|--------|--------|---------|-------|\n");
        for (index, row) in rows.iter().enumerate() {
            out.push_str(&format!(
                "| {} | {} | {} | {} | {} |\n",
                index + 1,
                row_status_badge(row),
                signal_kind_label(row),
                summarize_row(row),
                delta_label(row),
            ));
        }
        out.push('\n');

        let offenders: Vec<(&SignalRow, Vec<String>)> = rows
            .iter()
            .map(|row| (*row, offender_lines(row, offenders_limit)))
            .filter(|(_, lines)| !lines.is_empty())
            .collect();
        if !offenders.is_empty() {
            out.push_str("<details>\n<summary>Offenders</summary>\n\n");
            for (row, lines) in offenders {
                out.push_str(&format!("**{}**\n\n", signal_kind_label(row)));
                for line in lines {
                    out.push_str(&format!("- {line}\n"));
                }
                out.push('\n');
            }
            out.push_str("</details>\n\n");
        }
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DeltaStatus {
    Changed,
    Unchanged,
    NoPrevious,
    Unknown,
}

fn delta_status(row: &SignalRow) -> DeltaStatus {
    let Some(changes) = row
        .delta_vs_previous
        .as_ref()
        .and_then(|delta| delta.changes.as_object())
    else {
        return DeltaStatus::Unknown;
    };
    let status = changes.get("status").and_then(Value::as_str);
    match status {
        Some("changed") => DeltaStatus::Changed,
        Some("unchanged") => DeltaStatus::Unchanged,
        Some("no_previous_target" | "no_previous_run") => DeltaStatus::NoPrevious,
        _ => DeltaStatus::Unknown,
    }
}

fn delta_label(row: &SignalRow) -> &'static str {
    match delta_status(row) {
        DeltaStatus::Changed => "changed",
        DeltaStatus::Unchanged => "unchanged",
        DeltaStatus::NoPrevious => "new",
        DeltaStatus::Unknown => "—",
    }
}

fn row_status_label(row: &SignalRow) -> &'static str {
    if !row.pass {
        "fail"
    } else if has_warn_offenders(&row.offenders) {
        "warn"
    } else {
        "pass"
    }
}

fn row_status_badge(row: &SignalRow) -> String {
    let label = row_status_label(row);
    let image_url = match label {
        "pass" => PASS_IMAGE_URL,
        "warn" => WARN_IMAGE_URL,
        "fail" => FAIL_IMAGE_URL,
        _ => unreachable!("row_status_label returns a closed set"),
    };
    format!(r#"<img src="{image_url}" alt="{label}" width="20" height="20"> {label}"#)
}

fn signal_kind_label(row: &SignalRow) -> &'static str {
    match row.result {
        SignalResult::Test(_) => "test",
        SignalResult::Coverage(_) => "coverage",
        SignalResult::Size(_) => "size",
        SignalResult::Complexity(_) => "complexity",
        SignalResult::Deps(_) => "deps",
        SignalResult::Mutation(_) => "mutation",
    }
}

fn summarize_row(row: &SignalRow) -> String {
    match &row.result {
        SignalResult::Test(result) => format!(
            "total={} passed={} failed={}",
            result.total_tests, result.passed, result.failed
        ),
        SignalResult::Coverage(result) => format!(
            "percent={} status={}",
            result
                .headline_percent()
                .map(|value| format!("{value:.1}%"))
                .unwrap_or_else(|| String::from("—")),
            result.status
        ),
        SignalResult::Size(result) => format!(
            "max_lines={} files={} fail_count={}",
            result.max_lines, result.total_files, result.fail_count
        ),
        SignalResult::Complexity(result) => format!(
            "functions={} max_cyclo={:.1} fail_count={}",
            result.measured_functions, result.max_fn_cyclomatic, result.fail_count
        ),
        SignalResult::Deps(result) => format!(
            "crates={} edges={} violations={}",
            result.crate_count, result.edge_count, result.violation_count
        ),
        SignalResult::Mutation(result) => format!(
            "killed={} survived={} score={}",
            result.killed,
            result.survived,
            result
                .score
                .map(|value| format!("{value:.1}%"))
                .unwrap_or_else(|| String::from("—"))
        ),
    }
}

fn offender_lines(row: &SignalRow, offenders_limit: usize) -> Vec<String> {
    let mut lines = Vec::new();
    match &row.offenders {
        Offenders::Test(items) => {
            for item in items.iter().take(offenders_limit) {
                lines.push(format!(
                    "FAIL {} {}",
                    item.test_name.as_deref().unwrap_or("<unnamed-test>"),
                    item.message
                ));
            }
        }
        Offenders::Coverage(items) => {
            for item in items.iter().take(offenders_limit) {
                lines.push(format!(
                    "{} {} {:.1}%",
                    level_label(item.level),
                    item.file,
                    item.value
                ));
            }
        }
        Offenders::Size(items) => {
            for item in items.iter().take(offenders_limit) {
                lines.push(format!(
                    "{} {} lines={} warn={} fail={}",
                    level_label(item.level),
                    item.file,
                    item.value,
                    item.warn,
                    item.fail
                ));
            }
        }
        Offenders::Complexity(items) => {
            for item in items.iter().take(offenders_limit) {
                lines.push(format!(
                    "{} {}:{} {} cyclo={:.1}",
                    level_label(item.level),
                    item.file,
                    item.line,
                    item.function,
                    item.cyclomatic
                ));
            }
        }
        Offenders::Deps(items) => {
            for item in items.iter().take(offenders_limit) {
                lines.push(format!(
                    "{} {} -> {} (rule={})",
                    level_label(item.level),
                    item.from,
                    item.to,
                    item.rule
                ));
            }
        }
        Offenders::Mutation(items) => {
            for item in items.iter().take(offenders_limit) {
                lines.push(format!(
                    "{} {} {}",
                    level_label(item.level),
                    item.mutation_kind,
                    item.message
                ));
            }
        }
    }
    lines
}

fn level_label(level: Level) -> &'static str {
    match level {
        Level::Warn => "WARN",
        Level::Fail => "FAIL",
    }
}

fn has_warn_offenders(offenders: &Offenders) -> bool {
    match offenders {
        Offenders::Coverage(items) => items.iter().any(|item| item.level == Level::Warn),
        Offenders::Size(items) => items.iter().any(|item| item.level == Level::Warn),
        Offenders::Complexity(items) => items.iter().any(|item| item.level == Level::Warn),
        Offenders::Deps(items) => items.iter().any(|item| item.level == Level::Warn),
        Offenders::Mutation(items) => items.iter().any(|item| item.level == Level::Warn),
        Offenders::Test(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::build_markdown;
    use ayni_core::{
        Budget, CoverageOffender, CoverageResult, Delta, Language, Level, Offenders, RunArtifact,
        Scope, SignalKind, SignalResult, SignalRow,
    };
    use serde_json::json;

    #[test]
    fn build_markdown_renders_grouped_table() {
        let artifact = RunArtifact {
            schema_version: String::from("0.1.0"),
            rows: vec![SignalRow {
                kind: SignalKind::Coverage,
                language: Language::Rust,
                scope: Scope::default(),
                pass: false,
                result: SignalResult::Coverage(CoverageResult {
                    percent: Some(41.0),
                    line_percent: Some(41.0),
                    branch_percent: None,
                    engine: String::from("cargo-llvm-cov"),
                    status: String::from("ok"),
                }),
                budget: Budget::Coverage(json!({"line_percent_fail": 50.0})),
                offenders: Offenders::Coverage(vec![CoverageOffender {
                    file: String::from("src/lib.rs"),
                    line: Some(10),
                    value: 41.0,
                    level: Level::Fail,
                }]),
                delta_vs_previous: Some(Delta {
                    changes: json!({
                        "status": "changed",
                        "metrics": { "percent": { "from": 43.0, "to": 41.0, "delta": -2.0 } }
                    }),
                }),
                delta_vs_baseline: None,
            }],
        };

        let text = build_markdown(&artifact, 3);
        assert!(text.contains("# ayni analyze"));
        assert!(text.contains("## rust (workspace)"));
        assert!(text.contains("| # | Status | Signal | Summary | Delta |"));
        assert!(text.contains(
            r#"| 1 | <img src="https://raw.githubusercontent.com/gdurandvadas/ayni/refs/heads/main/assets/fail.svg" alt="fail" width="20" height="20"> fail | coverage |"#
        ));
        assert!(text.contains("| changed |"));
        assert!(text.contains("FAIL src/lib.rs 41.0%"));
    }
}
