use super::{AGENTS_MANAGED_BEGIN, AGENTS_MANAGED_END, LanguageArg, annotate_deltas_vs_previous};
use crate::install::{
    default_policy_toml, install_impl, upsert_managed_block, validate_install_foundation,
};
use ayni_core::{
    AyniPolicy, Budget, ExecutionResolution, Language, Offenders, RunArtifact, RunContext, Scope,
    SignalKind, SignalResult, TestResult,
};
use serde_json::json;
use std::fs;
use std::path::PathBuf;
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
    assert_eq!(LanguageArg::Python.as_language(), Language::Python);
}

#[test]
fn default_policy_templates_are_valid_for_each_language() {
    for language in [
        Language::Rust,
        Language::Go,
        Language::Node,
        Language::Python,
    ] {
        let policy: ayni_core::AyniPolicy =
            toml::from_str(&default_policy_toml(Some(language))).expect("policy");

        assert_eq!(policy.enabled_languages().expect("languages"), [language]);
        assert_eq!(policy.roots_for(language), ["."]);
        assert!(!policy.size_rules_for(language).is_empty());
    }
}

#[test]
fn default_policy_template_falls_back_to_rust() {
    let policy: ayni_core::AyniPolicy = toml::from_str(&default_policy_toml(None)).expect("policy");

    assert_eq!(
        policy.enabled_languages().expect("languages"),
        [Language::Rust]
    );
    assert_eq!(policy.roots_for(Language::Rust), ["."]);
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

    let targets = super::build_analyze_targets(
        dir.path(),
        &policy,
        None,
        None,
        Some(Language::Python),
        false,
    )
    .expect("targets");
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].language, Language::Python);
    assert_eq!(targets[0].run_context.execution.runner, "python");
    assert_eq!(targets[0].run_context.execution.kind, "direct_root");
}

#[test]
fn foundation_validation_creates_generic_artifact_work_dir() {
    let dir = TempDir::new().expect("tempdir");
    fs::write(
        dir.path().join("Cargo.toml"),
        r#"[package]
name = "fixture"
version = "0.1.0"
edition = "2021"
"#,
    )
    .expect("cargo manifest");
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
enabled = ["rust"]

[rust]
roots = ["."]

[rust.size]
"*.rs" = { warn = 400, fail = 800 }
"#,
    )
    .expect("policy");

    let failures = validate_install_foundation(dir.path(), &policy, Some(Language::Rust));

    assert!(failures.is_empty());
    assert!(dir.path().join(".ayni/work/rust/workspace").is_dir());
}

#[test]
fn install_new_python_policy_uses_member_roots_for_uv_workspace() {
    let dir = TempDir::new().expect("tempdir");
    fs::write(
        dir.path().join("pyproject.toml"),
        r#"[tool.uv.workspace]
members = ["packages/*", "services/*"]
exclude = ["services/agent-runtime"]
"#,
    )
    .expect("root pyproject");
    fs::create_dir_all(dir.path().join("packages/config")).expect("config dir");
    fs::write(
        dir.path().join("packages/config/pyproject.toml"),
        "[project]\nname='config'\n",
    )
    .expect("config pyproject");
    fs::create_dir_all(dir.path().join("services/api")).expect("api dir");
    fs::write(
        dir.path().join("services/api/pyproject.toml"),
        "[project]\nname='api'\n",
    )
    .expect("api pyproject");
    fs::create_dir_all(dir.path().join("services/agent-runtime")).expect("runtime dir");
    fs::write(
        dir.path().join("services/agent-runtime/pyproject.toml"),
        "[project]\nname='runtime'\n",
    )
    .expect("runtime pyproject");

    install_impl(&dir.path().to_string_lossy(), Some(Language::Python), false).expect("install");
    let policy = ayni_core::AyniPolicy::load(dir.path()).expect("policy");

    assert_eq!(
        policy.roots_for(Language::Python),
        ["packages/config", "services/api"]
    );
}

#[test]
fn install_existing_policy_does_not_rewrite_roots() {
    let dir = TempDir::new().expect("tempdir");
    fs::write(
        dir.path().join(".ayni.toml"),
        r#"[checks]
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
    fs::write(
        dir.path().join("pyproject.toml"),
        r#"[tool.uv.workspace]
members = ["packages/*"]
"#,
    )
    .expect("root pyproject");
    fs::create_dir_all(dir.path().join("packages/config")).expect("config dir");
    fs::write(
        dir.path().join("packages/config/pyproject.toml"),
        "[project]\nname='config'\n",
    )
    .expect("config pyproject");

    install_impl(&dir.path().to_string_lossy(), Some(Language::Python), false).expect("install");
    let policy = ayni_core::AyniPolicy::load(dir.path()).expect("policy");

    assert_eq!(policy.roots_for(Language::Python), ["."]);
}

#[test]
fn collector_errors_are_preserved_as_failed_rows() {
    let policy: AyniPolicy = toml::from_str(
        r#"
[checks]
test = true
coverage = true
size = false
complexity = false
deps = false
mutation = false

[languages]
enabled = ["python"]
"#,
    )
    .expect("policy");
    let context = RunContext {
        repo_root: PathBuf::from("/repo"),
        target_root: PathBuf::from("/repo/packages/api"),
        workdir: PathBuf::from("/repo/packages/api"),
        policy,
        scope: Scope {
            workspace_root: String::from("/repo"),
            path: Some(String::from("packages/api")),
            package: None,
            file: None,
        },
        diff: None,
        execution: ExecutionResolution::direct(
            "python",
            PathBuf::from("/repo/packages/api"),
            "test",
            100,
        ),
        debug: false,
    };

    let row = super::failed_signal_row(
        Language::Python,
        SignalKind::Coverage,
        &context,
        String::from("pytest-cov missing"),
    );

    assert!(!row.pass);
    assert_eq!(row.kind, SignalKind::Coverage);
    assert_eq!(row.scope.path.as_deref(), Some("packages/api"));
    match row.result {
        SignalResult::Coverage(result) => {
            assert_eq!(result.status, "error");
            let failure = result.failure.expect("failure");
            assert_eq!(failure.classification, "adapter_error");
            assert_eq!(failure.message, "pytest-cov missing");
        }
        other => panic!("unexpected result: {other:?}"),
    }
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
            failure: None,
        }),
        budget: Budget::Test(json!({})),
        offenders: Offenders::Test(Vec::new()),
        delta_vs_previous: None,
        delta_vs_baseline: None,
    }
}
