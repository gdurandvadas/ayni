use super::{
    AGENTS_MANAGED_BEGIN, AGENTS_MANAGED_END, annotate_deltas_vs_previous, upsert_managed_block,
};
use ayni_core::{
    Budget, Language, Offenders, RunArtifact, Scope, SignalKind, SignalResult, TestResult,
};
use serde_json::json;
use std::fs;
use tempfile::TempDir;

#[test]
fn upsert_managed_block_appends_when_missing() {
    let existing = "# Repository Rules\n\nKeep this text.\n";
    let managed = format!("{AGENTS_MANAGED_BEGIN}\n## Ayni\nx\n{AGENTS_MANAGED_END}\n");
    let updated = upsert_managed_block(existing, &managed);
    assert!(updated.contains("Keep this text."));
    assert!(updated.contains(AGENTS_MANAGED_BEGIN));
    assert!(updated.contains(AGENTS_MANAGED_END));
}

#[test]
fn upsert_managed_block_replaces_existing_managed_section() {
    let existing = format!("head\n\n{AGENTS_MANAGED_BEGIN}\nold\n{AGENTS_MANAGED_END}\n\ntail\n");
    let managed = format!("{AGENTS_MANAGED_BEGIN}\nnew\n{AGENTS_MANAGED_END}\n");
    let updated = upsert_managed_block(&existing, &managed);
    assert!(updated.contains("head"));
    assert!(updated.contains("tail"));
    assert!(updated.contains("\nnew\n"));
    assert!(!updated.contains("\nold\n"));
}

#[test]
fn upsert_managed_block_is_idempotent() {
    let managed = format!("{AGENTS_MANAGED_BEGIN}\n## Ayni\nx\n{AGENTS_MANAGED_END}\n");
    let once = upsert_managed_block("", &managed);
    let twice = upsert_managed_block(&once, &managed);
    assert_eq!(once, twice);
}

#[test]
fn annotate_deltas_vs_previous_marks_metric_and_pass_changes() {
    let previous = RunArtifact {
        schema_version: String::from("0.1.0"),
        rows: vec![test_row(false, 18, 2)],
    };
    let mut current = RunArtifact {
        schema_version: String::from("0.1.0"),
        rows: vec![test_row(true, 20, 0)],
    };

    annotate_deltas_vs_previous(&mut current, Some(&previous));
    let delta = current.rows[0]
        .delta_vs_previous
        .as_ref()
        .expect("delta is set");
    assert_eq!(delta.changes["status"], json!("changed"));
    assert_eq!(delta.changes["pass"]["from"], json!(false));
    assert_eq!(delta.changes["pass"]["to"], json!(true));
    assert_eq!(delta.changes["metrics"]["failed"]["delta"], json!(-2.0));
    assert_eq!(delta.changes["metrics"]["passed"]["delta"], json!(2.0));
}

#[test]
fn annotate_deltas_vs_previous_marks_missing_history() {
    let mut current = RunArtifact {
        schema_version: String::from("0.1.0"),
        rows: vec![test_row(true, 20, 0)],
    };

    annotate_deltas_vs_previous(&mut current, None);
    let delta = current.rows[0]
        .delta_vs_previous
        .as_ref()
        .expect("delta is set");
    assert_eq!(delta.changes["status"], json!("no_previous_run"));
}

#[test]
fn language_arg_accepts_python() {
    assert_eq!(super::LanguageArg::Python.as_language(), Language::Python);
}

#[test]
fn discover_python_roots_excludes_environment_dirs() {
    let dir = TempDir::new().expect("tempdir");
    fs::write(dir.path().join("pyproject.toml"), "").expect("root pyproject");
    fs::create_dir_all(dir.path().join("packages/api")).expect("api dir");
    fs::write(dir.path().join("packages/api/pyproject.toml"), "").expect("api pyproject");
    fs::create_dir_all(dir.path().join(".venv/lib")).expect("venv dir");
    fs::write(dir.path().join(".venv/lib/pyproject.toml"), "").expect("venv pyproject");

    assert_eq!(
        super::discover_python_roots(dir.path()),
        vec![String::from("."), String::from("packages/api")]
    );
}

#[test]
fn python_analyze_targets_are_built_when_enabled() {
    let dir = TempDir::new().expect("tempdir");
    fs::write(dir.path().join("pyproject.toml"), "").expect("pyproject");
    let policy: ayni_core::AyniPolicy = toml::from_str(
        r#"
[checks]
test = false
coverage = false
size = true
complexity = false
deps = false
mutation = false

[languages]
enabled = ["python"]

[python]
roots = ["."]

[python.size]
"**/*.py" = { warn = 400, fail = 800 }
"#,
    )
    .expect("policy");

    let targets =
        super::build_analyze_targets(dir.path(), &policy, None, None, Some(Language::Python))
            .expect("targets");
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].language, Language::Python);
}

fn test_row(pass: bool, passed: u64, failed: u64) -> ayni_core::SignalRow {
    ayni_core::SignalRow {
        kind: SignalKind::Test,
        language: Language::Rust,
        scope: Scope::default(),
        pass,
        result: SignalResult::Test(TestResult {
            total_tests: passed + failed,
            passed,
            failed,
            duration_ms: Some(400),
            runner: String::from("cargo-test"),
        }),
        budget: Budget::Test(json!({})),
        offenders: Offenders::Test(Vec::new()),
        delta_vs_previous: None,
        delta_vs_baseline: None,
    }
}
