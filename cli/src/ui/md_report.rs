use std::collections::BTreeMap;

use ayni_core::{FailureSummary, Level, Offenders, RunArtifact, SignalResult, SignalRow};

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

        out.push_str("| # | Signal | Summary | Status |\n");
        out.push_str("|---|--------|---------|--------|\n");
        for (index, row) in rows.iter().enumerate() {
            out.push_str(&format!(
                "| **{}** | **{}** | `{}` | {} |\n",
                index + 1,
                signal_kind_label(row),
                summarize_row(row),
                row_status_badge(row),
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
                out.push_str(&format!("{}\n", signal_kind_label(row)));
                for line in lines {
                    out.push_str(&format!("- {line}\n"));
                }
                out.push('\n');
            }
            out.push_str("</details>\n\n");
        }
    }
    render_failures(&mut out, artifact.failure_summaries());
    out
}

fn render_failures(out: &mut String, failures: Option<Vec<FailureSummary>>) {
    let Some(failures) = failures else {
        return;
    };

    out.push_str("## Failures\n\n");
    for failure in failures {
        out.push_str(&format!(
            "### {} ({})\n\n",
            signal_kind_label_from_summary(&failure),
            failure.language.as_str(),
        ));
        markdown_failure_field(out, "Classification", &failure.classification);
        markdown_failure_field(out, "Command", &failure.command);
        markdown_failure_field(out, "Working directory", &failure.cwd);
        if let Some(exit_code) = failure.exit_code {
            markdown_failure_field(out, "Exit code", &exit_code.to_string());
        }
        markdown_failure_field(out, "Message", &failure.message);
    }
}

fn signal_kind_label_from_summary(failure: &FailureSummary) -> &'static str {
    match failure.kind {
        ayni_core::SignalKind::Test => "test",
        ayni_core::SignalKind::Coverage => "coverage",
        ayni_core::SignalKind::Size => "size",
        ayni_core::SignalKind::Complexity => "complexity",
        ayni_core::SignalKind::Deps => "deps",
        ayni_core::SignalKind::Mutation => "mutation",
    }
}

fn markdown_failure_field(out: &mut String, label: &str, value: &str) {
    out.push_str(&format!(
        "**{label}:**\n\n{}\n\n",
        markdown_code_block(value)
    ));
}

fn markdown_code_block(value: &str) -> String {
    let fence = "`"
        .repeat(longest_backtick_run(value) + 1)
        .max("```".to_string());
    format!("{fence}text\n{value}\n{fence}")
}

fn longest_backtick_run(value: &str) -> usize {
    value
        .split(|character| character != '`')
        .map(str::len)
        .max()
        .unwrap_or(0)
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
                let location = item
                    .file
                    .as_deref()
                    .map(|file| format!("`{file}`"))
                    .unwrap_or_else(|| String::from("`<unnamed-test>`"));
                lines.push(format!(
                    "**FAIL** {} {} {}",
                    location,
                    item.test_name.as_deref().unwrap_or("<unnamed-test>"),
                    item.message
                ));
            }
        }
        Offenders::Coverage(items) => {
            for item in items.iter().take(offenders_limit) {
                lines.push(format!(
                    "**{}** `{}` {:.1}%",
                    level_label(item.level),
                    item.file,
                    item.value
                ));
            }
        }
        Offenders::Size(items) => {
            for item in items.iter().take(offenders_limit) {
                lines.push(format!(
                    "**{}** `{}` lines={} warn={} fail={}",
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
                    "**{}** `{}:{}` {} cyclo={:.1}",
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
                    "**{}** `{}` -> {} (rule={})",
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
                    "**{}** `{}` {}",
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
        Budget, CommandFailure, CoverageOffender, CoverageResult, Delta, Language, Level,
        Offenders, RunArtifact, Scope, SignalKind, SignalResult, SignalRow, TestFailure,
        TestResult,
    };
    use serde_json::json;

    #[test]
    fn build_markdown_renders_grouped_table() {
        let artifact = RunArtifact {
            schema_version: String::from("0.1.0"),
            metadata: Default::default(),
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
                    failure: None,
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
            }],
        };

        let text = build_markdown(&artifact, 3);
        assert!(text.contains("# ayni analyze"));
        assert!(text.contains("## rust (workspace)"));
        assert!(text.contains("| # | Signal | Summary | Status |"));
        assert!(text.contains(
            r#"| **1** | **coverage** | `percent=41.0% status=ok` | <img src="https://raw.githubusercontent.com/gdurandvadas/ayni/refs/heads/main/assets/fail.svg" alt="fail" width="20" height="20"> fail |"#
        ));
        assert!(text.contains("<details>\n<summary>Offenders</summary>\n\n"));
        assert!(text.contains("\ncoverage\n- "));
        assert!(text.contains("**FAIL** `src/lib.rs` 41.0%"));
        assert!(!text.contains("## Failures"));
    }

    #[test]
    fn build_markdown_renders_all_failures_without_truncating_them() {
        let artifact = RunArtifact {
            schema_version: String::from("0.2.0"),
            metadata: Default::default(),
            rows: vec![
                SignalRow {
                    kind: SignalKind::Test,
                    language: Language::Rust,
                    scope: Scope::default(),
                    pass: false,
                    result: SignalResult::Test(TestResult {
                        total_tests: 2,
                        passed: 0,
                        failed: 2,
                        duration_ms: None,
                        runner: String::from("cargo test"),
                        failure: Some(CommandFailure {
                            category: String::from("tool"),
                            classification: String::from("command_error"),
                            command: String::from("cargo test `weird`"),
                            cwd: String::from("/tmp/a[yni]"),
                            exit_code: Some(101),
                            message: String::from("failed *badly*\n```"),
                        }),
                    }),
                    budget: Budget::Test(json!({})),
                    offenders: Offenders::Test(vec![
                        TestFailure {
                            file: None,
                            line: None,
                            message: String::from("first"),
                            test_name: Some(String::from("first_failure")),
                        },
                        TestFailure {
                            file: None,
                            line: None,
                            message: String::from("second"),
                            test_name: Some(String::from("second_failure")),
                        },
                    ]),
                    delta_vs_previous: None,
                },
                SignalRow {
                    kind: SignalKind::Coverage,
                    language: Language::Rust,
                    scope: Scope::default(),
                    pass: false,
                    result: SignalResult::Coverage(CoverageResult {
                        percent: None,
                        line_percent: None,
                        branch_percent: None,
                        engine: String::from("coverage"),
                        status: String::from("failed"),
                        failure: Some(CommandFailure {
                            category: String::from("tool"),
                            classification: String::from("timeout"),
                            command: String::from("coverage run"),
                            cwd: String::from("/tmp/ayni"),
                            exit_code: None,
                            message: String::from("timed out"),
                        }),
                    }),
                    budget: Budget::Coverage(json!({})),
                    offenders: Offenders::Coverage(Vec::new()),
                    delta_vs_previous: None,
                },
            ],
        };

        let text = build_markdown(&artifact, 1);
        assert!(text.contains("first_failure"));
        assert!(!text.contains("second_failure"));
        assert!(text.contains("## Failures"));
        assert!(text.contains("### test (rust)"));
        assert!(text.contains("**Classification:**\n\n```text\ncommand_error"));
        assert!(text.contains("**Command:**\n\n```text\ncargo test `weird`"));
        assert!(text.contains("**Working directory:**\n\n```text\n/tmp/a[yni]"));
        assert!(text.contains("**Exit code:**\n\n```text\n101"));
        assert!(text.contains("**Message:**\n\n````text\nfailed *badly*\n```\n````"));
        let test_failure = text.find("### test (rust)").expect("test failure");
        let coverage_failure = text.find("### coverage (rust)").expect("coverage failure");
        assert!(test_failure < coverage_failure);
        let coverage_section = &text[coverage_failure..];
        assert!(!coverage_section.contains("**Exit code:**"));
    }
}
