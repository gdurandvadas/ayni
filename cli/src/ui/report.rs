use std::collections::BTreeMap;
#[cfg(test)]
use std::fs;
#[cfg(test)]
use std::path::Path;

#[cfg(test)]
use ayni_core::{AYNI_POLICY_FILE, AyniPolicy, RunArtifact};
use ayni_core::{
    Budget, ComplexityOffender, CoverageOffender, DepsOffender, Level, MutationOffender,
    SignalKind, SignalResult, SignalRow, SizeOffender, TestFailure,
};
use owo_colors::OwoColorize;
use serde_json::Value;

use crate::ui::color_enabled;

pub fn render_from_rows(rows: &[SignalRow], offenders_limit: usize, color: bool) -> String {
    build_report_text(rows, color, offenders_limit)
}

pub fn print_from_rows(rows: &[SignalRow], offenders_limit: usize) {
    let text = render_from_rows(rows, offenders_limit, color_enabled());
    println!("{text}");
}

#[cfg(test)]
pub fn print_from_run_artifact(signals_path: &Path) -> Result<(), String> {
    let content = fs::read_to_string(signals_path)
        .map_err(|e| format!("failed to read {}: {e}", signals_path.display()))?;
    let artifact: RunArtifact = serde_json::from_str(&content)
        .map_err(|e| format!("failed to parse {}: {e}", signals_path.display()))?;
    let offenders_limit = load_offenders_limit(signals_path);
    let text = render_from_rows(&artifact.rows, offenders_limit, color_enabled());
    println!("{text}");
    Ok(())
}

fn signal_kind_as_str(kind: SignalKind) -> &'static str {
    match kind {
        SignalKind::Test => "test",
        SignalKind::Coverage => "coverage",
        SignalKind::Size => "size",
        SignalKind::Complexity => "complexity",
        SignalKind::Deps => "deps",
        SignalKind::Mutation => "mutation",
    }
}

#[cfg(test)]
fn load_offenders_limit(signals_path: &Path) -> usize {
    let Some(root) = find_repo_root(signals_path) else {
        return usize::MAX;
    };

    match AyniPolicy::load(&root) {
        Ok(policy) => policy.report.offenders_limit,
        Err(error) => {
            eprintln!("warning: {error}; using default report.offenders_limit (unlimited)");
            usize::MAX
        }
    }
}

#[cfg(test)]
fn find_repo_root(start: &Path) -> Option<std::path::PathBuf> {
    for ancestor in start.ancestors() {
        if ancestor.join(AYNI_POLICY_FILE).is_file() {
            return Some(ancestor.to_path_buf());
        }
    }
    None
}

fn test_summary_from_rows(rows: &[SignalRow]) -> Option<(u64, u64, u64)> {
    for row in rows {
        if let SignalResult::Test(t) = &row.result {
            return Some((t.total_tests, t.passed, t.failed));
        }
    }
    None
}

fn build_report_text(rows: &[SignalRow], color: bool, offenders_limit: usize) -> String {
    let mut out = String::new();
    out.push('\n');
    out.push_str(&stylize(
        color,
        "ayni analyze report",
        Palette::Heading,
        true,
    ));
    out.push('\n');

    let mut grouped = BTreeMap::<String, Vec<&SignalRow>>::new();
    for row in rows {
        let root = row.scope.path.as_deref().unwrap_or(".");
        grouped
            .entry(format!("{}:{}", row.language.as_str(), root))
            .or_default()
            .push(row);
    }

    let mut total = 0usize;
    let mut pass = 0usize;
    for (group, lang_rows) in grouped {
        let mut parts = group.splitn(2, ':');
        let language = parts.next().unwrap_or("unknown");
        let root = parts.next().unwrap_or(".");
        let lang_total = lang_rows.len();
        let lang_pass = lang_rows.iter().filter(|r| r.pass).count();
        total += lang_total;
        pass += lang_pass;
        let root_label = if root == "." { "workspace" } else { root };
        let header = format!("{language} ({root_label})  {lang_pass}/{lang_total} passing");
        out.push_str(&stylize(color, &header, Palette::Section, true));
        out.push('\n');
        for row in lang_rows {
            let status = row_status(row);
            let summary = summarize(row);
            out.push_str(&format!(
                "  {} {} {:<12} {}",
                stylize(color, status.glyph(), status.palette(), true),
                stylize(color, status.label(), status.palette(), false),
                signal_kind_as_str(row.kind),
                summary
            ));
            out.push('\n');
            out.push_str(&offenders_text(color, row, offenders_limit));
        }
        out.push('\n');
    }

    out.push_str(&stylize(
        color,
        &format!("summary  {pass}/{total} checks passing"),
        Palette::Section,
        true,
    ));
    out.push('\n');
    if let Some((total_tests, passed_tests, failed_tests)) = test_summary_from_rows(rows) {
        out.push_str(&format!(
            "  tests: total={} passed={} failed={}\n",
            total_tests, passed_tests, failed_tests
        ));
    }
    out
}

fn summarize(row: &SignalRow) -> String {
    match &row.result {
        SignalResult::Test(result) => format!(
            "measured total={} passed={} failed={}",
            result.total_tests, result.passed, result.failed
        ),
        SignalResult::Coverage(result) => {
            let budget = match &row.budget {
                Budget::Coverage(budget) => Some(budget),
                _ => None,
            };
            let measured = result
                .headline_percent()
                .map(format_percent)
                .unwrap_or_else(|| String::from("—"));
            let warn = budget
                .and_then(|budget| budget.get("line_percent_warn"))
                .and_then(Value::as_f64);
            let fail = budget
                .and_then(|budget| budget.get("line_percent_fail"))
                .and_then(Value::as_f64);
            format!(
                "measured={} thresholds={} deltas={} engine={} status={}",
                measured,
                threshold_summary(warn, fail),
                delta_summary(result.headline_percent(), warn, fail),
                result.engine,
                result.status
            )
        }
        SignalResult::Size(result) => format!(
            "measured max_lines={} files={} warn_count={} fail_count={}",
            result.max_lines, result.total_files, result.warn_count, result.fail_count
        ),
        SignalResult::Complexity(result) => {
            let budget = match &row.budget {
                Budget::Complexity(budget) => Some(budget),
                _ => None,
            };
            let cyclo_warn =
                budget.and_then(|budget| nested_budget_number(budget, "fn_cyclomatic", "warn"));
            let cyclo_fail =
                budget.and_then(|budget| nested_budget_number(budget, "fn_cyclomatic", "fail"));
            let cognitive_warn =
                budget.and_then(|budget| nested_budget_number(budget, "fn_cognitive", "warn"));
            let cognitive_fail =
                budget.and_then(|budget| nested_budget_number(budget, "fn_cognitive", "fail"));
            let cognitive = result
                .max_fn_cognitive
                .map(|value| {
                    format!(
                        " max_cog={} cog_thresholds={} cog_deltas={}",
                        format_number(value),
                        threshold_summary(cognitive_warn, cognitive_fail),
                        delta_summary(Some(value), cognitive_warn, cognitive_fail)
                    )
                })
                .unwrap_or_default();
            format!(
                "measured functions={} max_cyclo={} cyclo_thresholds={} cyclo_deltas={} warn_count={} fail_count={}{}",
                result.measured_functions,
                format_number(result.max_fn_cyclomatic),
                threshold_summary(cyclo_warn, cyclo_fail),
                delta_summary(Some(result.max_fn_cyclomatic), cyclo_warn, cyclo_fail),
                result.warn_count,
                result.fail_count,
                cognitive
            )
        }
        SignalResult::Deps(result) => format!(
            "measured crates={} edges={} violations={}",
            result.crate_count, result.edge_count, result.violation_count
        ),
        SignalResult::Mutation(result) => format!(
            "measured score={} killed={} survived={} timeout={} engine={}",
            result
                .score
                .map(format_percent)
                .unwrap_or_else(|| String::from("—")),
            result.killed,
            result.survived,
            result.timeout,
            result.engine
        ),
    }
}

fn offenders_text(color: bool, row: &SignalRow, offenders_limit: usize) -> String {
    let mut out = String::new();
    match &row.offenders {
        ayni_core::Offenders::Test(items) => render_lines(
            &mut out,
            color,
            items.iter().map(test_failure_line).collect(),
            offenders_limit,
        ),
        ayni_core::Offenders::Coverage(items) => render_lines(
            &mut out,
            color,
            items.iter().map(coverage_offender_line).collect(),
            offenders_limit,
        ),
        ayni_core::Offenders::Size(items) => render_lines(
            &mut out,
            color,
            items.iter().map(size_offender_line).collect(),
            offenders_limit,
        ),
        ayni_core::Offenders::Complexity(items) => render_lines(
            &mut out,
            color,
            items.iter().map(complexity_offender_line).collect(),
            offenders_limit,
        ),
        ayni_core::Offenders::Deps(items) => render_lines(
            &mut out,
            color,
            items.iter().map(deps_offender_line).collect(),
            offenders_limit,
        ),
        ayni_core::Offenders::Mutation(items) => render_lines(
            &mut out,
            color,
            items.iter().map(mutation_offender_line).collect(),
            offenders_limit,
        ),
    }
    out
}

fn render_lines(
    out: &mut String,
    color: bool,
    lines: Vec<(Palette, String)>,
    offenders_limit: usize,
) {
    if lines.is_empty() {
        return;
    }
    let limit = offenders_limit.min(lines.len());
    for (palette, line) in lines.into_iter().take(limit) {
        out.push_str(&stylize(color, &format!("      {line}"), palette, false));
        out.push('\n');
    }
}

fn test_failure_line(failure: &TestFailure) -> (Palette, String) {
    (
        Palette::Failure,
        format!(
            "FAIL {} {} {}",
            failure.test_name.as_deref().unwrap_or("<unnamed-test>"),
            format_optional_location(failure.file.as_deref(), failure.line),
            failure.message
        ),
    )
}

fn coverage_offender_line(offender: &CoverageOffender) -> (Palette, String) {
    let location = format_location(&offender.file, offender.line);
    let level = level_label(offender.level);
    (
        palette_for_level(offender.level),
        format!(
            "{} {} {} {}",
            level,
            location,
            format_percent(offender.value),
            level.to_ascii_lowercase()
        ),
    )
}

fn size_offender_line(offender: &SizeOffender) -> (Palette, String) {
    (
        palette_for_level(offender.level),
        format!(
            "{} {} lines={} (warn={} fail={})",
            level_label(offender.level),
            offender.file,
            offender.value,
            offender.warn,
            offender.fail
        ),
    )
}

fn complexity_offender_line(offender: &ComplexityOffender) -> (Palette, String) {
    let cognitive = offender
        .cognitive
        .map(|value| format!(" cog={}", format_number(value)))
        .unwrap_or_default();
    (
        palette_for_level(offender.level),
        format!(
            "{} {}:{} {} cyclo={}{} {}",
            level_label(offender.level),
            offender.file,
            offender.line,
            offender.function,
            format_number(offender.cyclomatic),
            cognitive,
            level_label(offender.level).to_ascii_lowercase()
        ),
    )
}

fn deps_offender_line(offender: &DepsOffender) -> (Palette, String) {
    (
        palette_for_level(offender.level),
        format!(
            "{} {} -> {} (rule={})",
            level_label(offender.level),
            offender.from,
            offender.to,
            offender.rule
        ),
    )
}

fn mutation_offender_line(offender: &MutationOffender) -> (Palette, String) {
    (
        palette_for_level(offender.level),
        format!(
            "{} {} {} {}",
            level_label(offender.level),
            format_optional_location(offender.file.as_deref(), offender.line),
            offender.mutation_kind,
            offender.message
        ),
    )
}

fn format_optional_location(file: Option<&str>, line: Option<u64>) -> String {
    match file {
        Some(file) => format_location(file, line),
        None => String::from("<unknown>"),
    }
}

fn format_location(file: &str, line: Option<u64>) -> String {
    line.map(|line| format!("{file}:{line}"))
        .unwrap_or_else(|| file.to_string())
}

fn level_label(level: Level) -> &'static str {
    match level {
        Level::Warn => "WARN",
        Level::Fail => "FAIL",
    }
}

fn palette_for_level(level: Level) -> Palette {
    match level {
        Level::Warn => Palette::Warning,
        Level::Fail => Palette::Failure,
    }
}

fn nested_budget_number(value: &Value, key: &str, nested: &str) -> Option<f64> {
    value.get(key)?.get(nested)?.as_f64()
}

fn threshold_summary(warn: Option<f64>, fail: Option<f64>) -> String {
    format!(
        "warn={} fail={}",
        warn.map(format_number).unwrap_or_else(|| String::from("—")),
        fail.map(format_number).unwrap_or_else(|| String::from("—"))
    )
}

fn delta_summary(measured: Option<f64>, warn: Option<f64>, fail: Option<f64>) -> String {
    let Some(measured) = measured else {
        return String::from("warn=— fail=—");
    };
    format!(
        "warn={} fail={}",
        delta_text(measured, warn),
        delta_text(measured, fail)
    )
}

fn delta_text(measured: f64, threshold: Option<f64>) -> String {
    threshold
        .map(|threshold| format_signed(measured - threshold))
        .unwrap_or_else(|| String::from("—"))
}

fn format_signed(value: f64) -> String {
    if value >= 0.0 {
        format!("+{value:.1}")
    } else {
        format!("{value:.1}")
    }
}

fn format_percent(value: f64) -> String {
    format!("{value:.1}%")
}

fn format_number(value: f64) -> String {
    format!("{value:.1}")
}

#[derive(Clone, Copy)]
enum RowStatus {
    Pass,
    Warn,
    Fail,
}

impl RowStatus {
    fn glyph(self) -> &'static str {
        match self {
            Self::Pass => "✓",
            Self::Warn => "!",
            Self::Fail => "✗",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Warn => "warn",
            Self::Fail => "fail",
        }
    }

    fn palette(self) -> Palette {
        match self {
            Self::Pass => Palette::Success,
            Self::Warn => Palette::Warning,
            Self::Fail => Palette::Failure,
        }
    }
}

fn row_status(row: &SignalRow) -> RowStatus {
    if !row.pass {
        return RowStatus::Fail;
    }
    match &row.result {
        SignalResult::Size(result) if result.warn_count > 0 => RowStatus::Warn,
        SignalResult::Complexity(result) if result.warn_count > 0 => RowStatus::Warn,
        SignalResult::Coverage(_) if has_warn_offenders(&row.offenders) => RowStatus::Warn,
        SignalResult::Mutation(result) if result.timeout > 0 => RowStatus::Warn,
        _ if has_warn_offenders(&row.offenders) => RowStatus::Warn,
        _ => RowStatus::Pass,
    }
}

fn has_warn_offenders(offenders: &ayni_core::Offenders) -> bool {
    match offenders {
        ayni_core::Offenders::Coverage(items) => items.iter().any(|item| item.level == Level::Warn),
        ayni_core::Offenders::Size(items) => items.iter().any(|item| item.level == Level::Warn),
        ayni_core::Offenders::Complexity(items) => {
            items.iter().any(|item| item.level == Level::Warn)
        }
        ayni_core::Offenders::Deps(items) => items.iter().any(|item| item.level == Level::Warn),
        ayni_core::Offenders::Mutation(items) => items.iter().any(|item| item.level == Level::Warn),
        ayni_core::Offenders::Test(_) => false,
    }
}

#[derive(Clone, Copy)]
enum Palette {
    Heading,
    Section,
    Success,
    Failure,
    Warning,
}

fn stylize(color_enabled: bool, value: &str, palette: Palette, bold: bool) -> String {
    if !color_enabled {
        return value.to_owned();
    }
    match (palette, bold) {
        (Palette::Heading, true) => value.bold().bright_blue().to_string(),
        (Palette::Section, true) => value.bold().bright_white().to_string(),
        (Palette::Success, true) => value.bold().green().to_string(),
        (Palette::Failure, true) => value.bold().red().to_string(),
        (Palette::Warning, true) => value.bold().yellow().to_string(),
        (Palette::Heading, false) => value.bright_blue().to_string(),
        (Palette::Section, false) => value.bright_white().to_string(),
        (Palette::Success, false) => value.green().to_string(),
        (Palette::Failure, false) => value.red().to_string(),
        (Palette::Warning, false) => value.yellow().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ayni_core::{
        AYNI_POLICY_FILE, AYNI_SIGNAL_SCHEMA_VERSION, Budget, ComplexityOffender, ComplexityResult,
        CoverageOffender, CoverageResult, DepsResult, Language, Offenders, Scope, SignalKind,
        SignalResult, SignalRow,
    };
    use serde_json::json;
    use tempfile::TempDir;

    #[test]
    fn build_report_text_groups_rows_by_language() {
        let rows = vec![
            SignalRow {
                kind: SignalKind::Size,
                language: Language::Rust,
                scope: Scope {
                    path: Some(String::from("apps/api")),
                    ..Scope::default()
                },
                pass: false,
                result: SignalResult::Size(ayni_core::SizeResult {
                    max_lines: 900,
                    total_files: 3,
                    warn_count: 0,
                    fail_count: 1,
                }),
                budget: Budget::Size(serde_json::json!({})),
                offenders: Offenders::Size(vec![ayni_core::SizeOffender {
                    file: String::from("cli/src/main.rs"),
                    value: 900,
                    warn: 400,
                    fail: 700,
                    level: Level::Fail,
                }]),
                delta_vs_previous: None,
                delta_vs_baseline: None,
            },
            SignalRow {
                kind: SignalKind::Deps,
                language: Language::Node,
                scope: Scope::default(),
                pass: true,
                result: SignalResult::Deps(DepsResult {
                    crate_count: 3,
                    edge_count: 1,
                    violation_count: 0,
                }),
                budget: Budget::Deps(serde_json::json!({})),
                offenders: Offenders::Deps(Vec::new()),
                delta_vs_previous: None,
                delta_vs_baseline: None,
            },
        ];
        let text = build_report_text(&rows, false, 4);
        assert!(text.contains("rust (apps/api)  0/1 passing"));
        assert!(text.contains("node (workspace)  1/1 passing"));
        assert!(text.contains("summary  1/2 checks passing"));
    }

    #[test]
    fn build_report_text_respects_offenders_limit_and_renders_thresholds() {
        let rows = vec![
            SignalRow {
                kind: SignalKind::Coverage,
                language: Language::Rust,
                scope: Scope::default(),
                pass: true,
                result: SignalResult::Coverage(CoverageResult {
                    percent: Some(68.0),
                    line_percent: Some(68.0),
                    branch_percent: None,
                    engine: String::from("cargo-llvm-cov"),
                    status: String::from("ok"),
                }),
                budget: Budget::Coverage(serde_json::json!({
                    "line_percent_warn": 70.0,
                    "line_percent_fail": 50.0
                })),
                offenders: Offenders::Coverage(vec![
                    CoverageOffender {
                        file: String::from("a.rs"),
                        line: Some(10),
                        value: 68.0,
                        level: Level::Warn,
                    },
                    CoverageOffender {
                        file: String::from("b.rs"),
                        line: Some(11),
                        value: 67.0,
                        level: Level::Warn,
                    },
                    CoverageOffender {
                        file: String::from("c.rs"),
                        line: Some(12),
                        value: 66.0,
                        level: Level::Warn,
                    },
                ]),
                delta_vs_previous: None,
                delta_vs_baseline: None,
            },
            SignalRow {
                kind: SignalKind::Complexity,
                language: Language::Rust,
                scope: Scope::default(),
                pass: true,
                result: SignalResult::Complexity(ComplexityResult {
                    engine: String::from("rust-code-analysis-cli"),
                    method: String::from("ast_metrics"),
                    measured_functions: 10,
                    max_fn_cyclomatic: 11.0,
                    max_fn_cognitive: Some(16.0),
                    warn_count: 1,
                    fail_count: 0,
                }),
                budget: Budget::Complexity(serde_json::json!({
                    "fn_cyclomatic": {"warn": 10.0, "fail": 20.0},
                    "fn_cognitive": {"warn": 15.0, "fail": 25.0}
                })),
                offenders: Offenders::Complexity(vec![ComplexityOffender {
                    file: String::from("core/src/lib.rs"),
                    line: 42,
                    function: String::from("alpha"),
                    cyclomatic: 11.0,
                    cognitive: Some(16.0),
                    level: Level::Warn,
                }]),
                delta_vs_previous: None,
                delta_vs_baseline: None,
            },
        ];

        let text = build_report_text(&rows, false, 2);
        assert!(text.contains("thresholds=warn=70.0 fail=50.0"));
        assert!(text.contains("deltas=warn=-2.0 fail=+18.0"));
        assert!(text.contains("cyclo_thresholds=warn=10.0 fail=20.0"));
        assert!(text.contains("cog_deltas=warn=+1.0 fail=-9.0"));
        assert!(text.contains("WARN a.rs:10 68.0% warn"));
        assert!(text.contains("WARN b.rs:11 67.0% warn"));
        assert!(!text.contains("WARN c.rs:12 66.0% warn"));
    }

    #[test]
    fn print_from_run_artifact_roundtrip() {
        let dir = TempDir::new().expect("tempdir");
        let signals_path = dir.path().join("signals.json");

        let artifact = RunArtifact {
            schema_version: String::from(AYNI_SIGNAL_SCHEMA_VERSION),
            rows: vec![SignalRow {
                kind: SignalKind::Deps,
                language: Language::Rust,
                scope: Scope::default(),
                pass: true,
                result: SignalResult::Deps(DepsResult {
                    crate_count: 2,
                    edge_count: 1,
                    violation_count: 0,
                }),
                budget: Budget::Deps(json!({})),
                offenders: Offenders::Deps(vec![]),
                delta_vs_previous: None,
                delta_vs_baseline: None,
            }],
        };
        let body = serde_json::to_string_pretty(&artifact).expect("serialize");
        fs::write(&signals_path, body).expect("write signals");

        let result = print_from_run_artifact(&signals_path);
        assert!(result.is_ok());
    }

    #[test]
    fn load_offenders_limit_defaults_when_policy_parse_fails() {
        let dir = TempDir::new().expect("tempdir");
        let artifacts_dir = dir.path().join(".ayni/last");
        fs::create_dir_all(&artifacts_dir).expect("artifacts dir");
        fs::write(
            dir.path().join(AYNI_POLICY_FILE),
            "[report\noffenders_limit = 3",
        )
        .expect("write invalid policy");

        let signals_path = artifacts_dir.join("signals.json");
        let limit = load_offenders_limit(&signals_path);

        assert_eq!(limit, usize::MAX);
    }
}
