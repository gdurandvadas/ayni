#[cfg(test)]
use crate::policy::load_from_path;
#[cfg(test)]
use std::fs;
#[cfg(test)]
use std::path::Path;

#[cfg(test)]
use ayni_core::AYNI_POLICY_FILE;
use ayni_core::{
    Budget, CommandFailure, CompletionScope, ComplexityOffender, CoverageOffender, DepsOffender,
    Level, MutationOffender, RunArtifact, SignalResult, SignalRow, SizeOffender, TestFailure,
};
use owo_colors::OwoColorize;

use crate::ui::{
    FAIL_RGB, PASS_RGB, WARN_RGB, color_enabled,
    report_view::{
        ReportStatus, ReportView, completion_scope_label, completion_stage_label,
        completion_state_label, signal_kind_label,
    },
};

pub fn print_from_artifact(artifact: &RunArtifact, offenders_limit: usize) {
    let text = build_report_text(artifact, color_enabled(), offenders_limit);
    println!("{text}");
}

#[cfg(test)]
pub fn print_from_run_artifact(signals_path: &Path) -> Result<(), String> {
    let content = fs::read_to_string(signals_path)
        .map_err(|e| format!("failed to read {}: {e}", signals_path.display()))?;
    let artifact: RunArtifact = serde_json::from_str(&content)
        .map_err(|e| format!("failed to parse {}: {e}", signals_path.display()))?;
    let offenders_limit = load_offenders_limit(signals_path);
    let text = build_report_text(&artifact, color_enabled(), offenders_limit);
    println!("{text}");
    Ok(())
}

#[cfg(test)]
fn load_offenders_limit(signals_path: &Path) -> usize {
    let Some(root) = find_repo_root(signals_path) else {
        return usize::MAX;
    };

    match load_from_path(&root.join(AYNI_POLICY_FILE)) {
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

fn build_report_text(artifact: &RunArtifact, color: bool, offenders_limit: usize) -> String {
    let view = ReportView::new(artifact);
    let completion = &artifact.completion;
    let mut out = String::new();
    out.push('\n');
    let heading = match completion.scope {
        CompletionScope::Requested => "ayni verify report",
        CompletionScope::Repository => "ayni check report",
    };
    out.push_str(&stylize(color, heading, Palette::Heading, true));
    out.push('\n');

    out.push_str(&format!(
        "completion  scope={} state={} targets={}/{} detected={} skipped={}\n",
        completion_scope_label(completion.scope),
        completion_state_label(completion.state),
        completion.completed_targets,
        completion.expected_targets,
        completion.detected_targets,
        completion.skipped_targets,
    ));
    for issue in &completion.issues {
        out.push_str(&format!(
            "  incomplete language={} root={} stage={}: {}\n",
            issue.language.as_str(),
            issue.configured_root,
            completion_stage_label(issue.stage),
            issue.message,
        ));
    }
    out.push('\n');

    for group in &view.groups {
        let root_label = if group.root == "." {
            "workspace"
        } else {
            group.root.as_str()
        };
        let header = format!(
            "{} ({root_label})  {}/{} passing",
            group.language.as_str(),
            group.passing,
            group.rows.len()
        );
        out.push_str(&stylize(color, &header, Palette::Section, true));
        out.push('\n');
        for row in &group.rows {
            let status = ReportStatus::for_row(row);
            let summary = summarize(row);
            out.push_str(&format!(
                "  {} {} {:<12} {}",
                stylize(color, status.glyph(), palette_for_status(status), true),
                stylize(color, status.label(), palette_for_status(status), false),
                signal_kind_label(row.kind),
                summary
            ));
            out.push('\n');
            out.push_str(&offenders_text(color, row, offenders_limit));
        }
        out.push('\n');
    }

    out.push_str(&stylize(
        color,
        &format!("summary  {}/{} checks passing", view.passing, view.total),
        Palette::Section,
        true,
    ));
    out.push('\n');
    if let Some((total_tests, passed_tests, failed_tests)) = test_summary_from_rows(&artifact.rows)
    {
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
            "measured total={} passed={} failed={}{}",
            result.total_tests,
            result.passed,
            result.failed,
            failure_suffix(result.failure.as_ref())
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
            let warn = budget.and_then(|budget| budget.line_percent_warn);
            let fail = budget.and_then(|budget| budget.line_percent_fail);
            format!(
                "measured={} thresholds={} deltas={} engine={} status={}{}",
                measured,
                threshold_summary(warn, fail),
                delta_summary(result.headline_percent(), warn, fail),
                result.engine,
                result.status,
                failure_suffix(result.failure.as_ref())
            )
        }
        SignalResult::Size(result) => format!(
            "measured max_lines={} files={} warn_count={} fail_count={}{}",
            result.max_lines,
            result.total_files,
            result.warn_count,
            result.fail_count,
            failure_suffix(result.failure.as_ref())
        ),
        SignalResult::Complexity(result) => {
            let budget = match &row.budget {
                Budget::Complexity(budget) => Some(budget),
                _ => None,
            };
            let cyclo_warn =
                budget.and_then(|budget| budget.fn_cyclomatic.map(|threshold| threshold.warn));
            let cyclo_fail =
                budget.and_then(|budget| budget.fn_cyclomatic.map(|threshold| threshold.fail));
            let cognitive_warn =
                budget.and_then(|budget| budget.fn_cognitive.map(|threshold| threshold.warn));
            let cognitive_fail =
                budget.and_then(|budget| budget.fn_cognitive.map(|threshold| threshold.fail));
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
                "measured functions={} max_cyclo={} cyclo_thresholds={} cyclo_deltas={} warn_count={} fail_count={}{}{}",
                result.measured_functions,
                format_number(result.max_fn_cyclomatic),
                threshold_summary(cyclo_warn, cyclo_fail),
                delta_summary(Some(result.max_fn_cyclomatic), cyclo_warn, cyclo_fail),
                result.warn_count,
                result.fail_count,
                cognitive,
                failure_suffix(result.failure.as_ref())
            )
        }
        SignalResult::Deps(result) => format!(
            "measured crates={} edges={} violations={}{}",
            result.crate_count,
            result.edge_count,
            result.violation_count,
            failure_suffix(result.failure.as_ref())
        ),
        SignalResult::Mutation(result) => format!(
            "measured score={} killed={} survived={} timeout={} engine={}{}",
            result
                .score
                .map(format_percent)
                .unwrap_or_else(|| String::from("—")),
            result.killed,
            result.survived,
            result.timeout,
            result.engine,
            failure_suffix(result.failure.as_ref())
        ),
    }
}

fn offenders_text(color: bool, row: &SignalRow, offenders_limit: usize) -> String {
    let mut out = String::new();
    if let Some(failure) = command_failure_for_row(row) {
        render_lines(
            &mut out,
            color,
            vec![(Palette::Failure, command_failure_line(failure))],
            offenders_limit,
        );
    }
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

fn command_failure_for_row(row: &SignalRow) -> Option<&CommandFailure> {
    match &row.result {
        SignalResult::Test(result) => result.failure.as_ref(),
        SignalResult::Coverage(result) => result.failure.as_ref(),
        SignalResult::Size(result) => result.failure.as_ref(),
        SignalResult::Complexity(result) => result.failure.as_ref(),
        SignalResult::Deps(result) => result.failure.as_ref(),
        SignalResult::Mutation(result) => result.failure.as_ref(),
    }
}

fn failure_suffix(failure: Option<&CommandFailure>) -> String {
    failure
        .map(|failure| {
            format!(
                " failure={} category={}",
                failure.classification, failure.category
            )
        })
        .unwrap_or_default()
}

fn command_failure_line(failure: &CommandFailure) -> String {
    format!(
        "FAIL {} {} exit={} command=`{}` cwd={} {}",
        failure.category,
        failure.classification,
        failure
            .exit_code
            .map(|code| code.to_string())
            .unwrap_or_else(|| String::from("—")),
        failure.command,
        failure.cwd,
        failure.message.replace('\n', " ")
    )
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

fn palette_for_status(status: ReportStatus) -> Palette {
    match status {
        ReportStatus::Pass => Palette::Success,
        ReportStatus::Warn => Palette::Warning,
        ReportStatus::Fail => Palette::Failure,
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
    let apply_rgb = |text: &str, rgb: (u8, u8, u8), bold: bool| {
        if bold {
            text.bold().truecolor(rgb.0, rgb.1, rgb.2).to_string()
        } else {
            text.truecolor(rgb.0, rgb.1, rgb.2).to_string()
        }
    };
    match (palette, bold) {
        (Palette::Heading, true) => value.bold().bright_blue().to_string(),
        (Palette::Section, true) => value.bold().bright_white().to_string(),
        (Palette::Success, true) => apply_rgb(value, PASS_RGB, true),
        (Palette::Failure, true) => apply_rgb(value, FAIL_RGB, true),
        (Palette::Warning, true) => apply_rgb(value, WARN_RGB, true),
        (Palette::Heading, false) => value.bright_blue().to_string(),
        (Palette::Section, false) => value.bright_white().to_string(),
        (Palette::Success, false) => apply_rgb(value, PASS_RGB, false),
        (Palette::Failure, false) => apply_rgb(value, FAIL_RGB, false),
        (Palette::Warning, false) => apply_rgb(value, WARN_RGB, false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ayni_core::{
        AYNI_POLICY_FILE, AYNI_SIGNAL_SCHEMA_VERSION, Budget, ComplexityBudget, ComplexityOffender,
        ComplexityResult, CoverageBudget, CoverageOffender, CoverageResult, DepsBudget, DepsResult,
        Finding, FindingMetadata, Findings, FloatThresholdBudget, Language, Offenders,
        RunCompletion, Scope, SignalKind, SignalResult, SignalRow, SizeBudget, SizeOffender,
        VerificationMetadata,
    };
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
                    failure: None,
                }),
                budget: Budget::Size(SizeBudget::default()),
                offenders: Offenders::Size(vec![ayni_core::SizeOffender {
                    file: String::from("cli/src/main.rs"),
                    value: 900,
                    warn: 400,
                    fail: 700,
                    level: Level::Fail,
                }]),
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
                    failure: None,
                }),
                budget: Budget::Deps(DepsBudget::default()),
                offenders: Offenders::Deps(Vec::new()),
            },
        ];
        let artifact = RunArtifact {
            rows,
            ..RunArtifact::default()
        };
        let text = build_report_text(&artifact, false, 4);
        assert!(text.contains("rust (apps/api)  0/1 passing"));
        assert!(text.contains("node (workspace)  1/1 passing"));
        assert!(text.contains("summary  1/2 checks passing"));
    }

    #[test]
    fn build_report_text_renders_completion_scope_state_and_issue() {
        let completion = ayni_core::RunCompletion {
            scope: ayni_core::CompletionScope::Requested,
            state: ayni_core::CompletionState::Incomplete,
            expected_targets: 1,
            detected_targets: 0,
            completed_targets: 0,
            skipped_targets: 1,
            issues: vec![ayni_core::CompletionIssue {
                language: Language::Rust,
                configured_root: String::from("crates/api"),
                stage: ayni_core::CompletionStage::Detection,
                message: String::from("not detected"),
            }],
        };

        let artifact = RunArtifact {
            completion,
            ..RunArtifact::default()
        };
        let text = build_report_text(&artifact, false, 3);
        assert!(text.contains("ayni verify report"));
        assert!(text.contains("scope=requested state=incomplete"));
        assert!(text.contains("targets=0/1 detected=0 skipped=1"));
        assert!(
            text.contains("incomplete language=rust root=crates/api stage=detection: not detected")
        );
    }

    #[test]
    fn build_report_text_omits_verification_commands() {
        let finding = |id_character: char, command: &str| Finding {
            metadata: FindingMetadata {
                id: format!(
                    "ayni:finding:v1:sha256:{}",
                    id_character.to_string().repeat(64)
                ),
                verification: VerificationMetadata {
                    target: None,
                    command: Some(command.to_string()),
                },
            },
            offender: SizeOffender {
                file: String::from("src/lib.rs"),
                value: 10,
                warn: 5,
                fail: 9,
                level: Level::Fail,
            },
        };
        let command = "ayni verify size --file 'src/lib.rs'";
        let artifact = RunArtifact {
            findings: vec![Findings::Size(vec![
                finding('a', command),
                finding('b', command),
            ])],
            ..RunArtifact::default()
        };

        let text = build_report_text(&artifact, false, 2);

        assert!(!text.contains("verification commands"));
        assert!(!text.contains(command));
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
                    failure: None,
                }),
                budget: Budget::Coverage(CoverageBudget {
                    line_percent_warn: Some(70.0),
                    line_percent_fail: Some(50.0),
                    ..CoverageBudget::default()
                }),
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
                    failure: None,
                }),
                budget: Budget::Complexity(ComplexityBudget {
                    fn_cyclomatic: Some(FloatThresholdBudget {
                        warn: 10.0,
                        fail: 20.0,
                    }),
                    fn_cognitive: Some(FloatThresholdBudget {
                        warn: 15.0,
                        fail: 25.0,
                    }),
                }),
                offenders: Offenders::Complexity(vec![ComplexityOffender {
                    file: String::from("core/src/lib.rs"),
                    line: 42,
                    function: String::from("alpha"),
                    cyclomatic: 11.0,
                    cognitive: Some(16.0),
                    level: Level::Warn,
                }]),
            },
        ];

        let artifact = RunArtifact {
            rows,
            ..RunArtifact::default()
        };
        let text = build_report_text(&artifact, false, 2);
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
            metadata: Default::default(),
            completion: RunCompletion::complete(CompletionScope::Repository, 1),
            findings: Vec::new(),
            rows: vec![SignalRow {
                kind: SignalKind::Deps,
                language: Language::Rust,
                scope: Scope::default(),
                pass: true,
                result: SignalResult::Deps(DepsResult {
                    crate_count: 2,
                    edge_count: 1,
                    violation_count: 0,
                    failure: None,
                }),
                budget: Budget::Deps(DepsBudget::default()),
                offenders: Offenders::Deps(vec![]),
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
