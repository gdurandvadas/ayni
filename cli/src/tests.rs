use super::{SIGNALS_ARTIFACT, VERIFY_SIGNALS_ARTIFACT, persist_artifact_at, serialize_artifact};

#[test]
fn verify_artifact_does_not_replace_analyze_artifact() {
    assert_ne!(VERIFY_SIGNALS_ARTIFACT, SIGNALS_ARTIFACT);
    assert_eq!(VERIFY_SIGNALS_ARTIFACT, ".ayni/verify/last/signals.json");
}

#[test]
fn completion_planner_represents_an_undetected_configured_root() {
    let dir = TempDir::new().expect("tempdir");
    let policy: AyniPolicy = toml::from_str(
        r#"
[checks]
test = true
coverage = false
size = false
complexity = false
deps = false
mutation = false

[languages]
enabled = ["rust"]

[rust]
roots = ["missing"]
"#,
    )
    .expect("policy");

    let planning =
        super::build_analyze_targets(dir.path(), &policy, None, None, Some(Language::Rust), false)
            .expect("planning");

    assert_eq!(planning.expected_targets, 1);
    assert_eq!(planning.detected_targets, 0);
    assert!(planning.targets.is_empty());
    assert_eq!(planning.issues.len(), 1);
    assert_eq!(planning.issues[0].configured_root, "missing");
    assert_eq!(
        planning.issues[0].stage,
        ayni_core::CompletionStage::Detection
    );
}

#[test]
fn completion_artifact_writer_atomically_replaces_existing_evidence() {
    let dir = TempDir::new().expect("tempdir");
    let path = dir.path().join(SIGNALS_ARTIFACT);
    fs::create_dir_all(path.parent().expect("parent")).expect("artifact directory");
    fs::write(&path, "stale\n").expect("stale artifact");

    persist_artifact_at(dir.path(), SIGNALS_ARTIFACT, "current\n").expect("persist");

    assert_eq!(fs::read_to_string(path).expect("artifact"), "current\n");
}
use crate::agents::{MANAGED_BEGIN, MANAGED_END, managed_block, sync_impl, upsert_managed_block};
use ayni_core::{
    AYNI_SIGNAL_SCHEMA_VERSION, AyniPolicy, Budget, ExecutionResolution, InvocationContext,
    Language, Offenders, OutputContext, RunArtifact, RunArtifactMetadata, RunContext, Scope,
    SignalKind, SignalResult, TestResult,
};
use serde_json::json;
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

#[test]
fn agents_sync_creates_managed_file_when_absent() {
    let dir = TempDir::new().expect("tempdir");

    sync_impl(&dir.path().to_string_lossy()).expect("sync");

    assert_eq!(
        fs::read_to_string(dir.path().join("AGENTS.md")).expect("agents"),
        managed_block()
    );
}

#[test]
fn agents_managed_guidance_describes_discovery_policy_and_quality_workflow() {
    let managed = managed_block();

    for guidance in [
        "`ayni help`",
        "`ayni help <command> [subcommand]`",
        "`ayni <command> --help`",
        "`.ayni.toml` as the authoritative repository quality policy",
        "`ayni contract show`",
        "ayni verify <signal> [selectors]",
        "full repository analysis as the completion gate",
        "`.ayni/last/signals.json`",
        "narrowest supported `ayni verify <signal>`",
        "exact verification command supplied by a finding",
        "incomplete artifacts as failure",
        "never loosen `.ayni.toml` merely",
        "detailed, typed signal results",
        "completion state and target accounting",
        "exact verification command",
    ] {
        assert!(managed.contains(guidance), "missing guidance: {guidance}");
    }
    assert!(!managed.contains("ayni <command> help"));
    assert!(!managed.contains("schema-v2"));
    assert!(!managed.contains("deltas"));
    assert!(!managed.contains("and repair the listed offenders and\nand repair"));
}

#[test]
fn agents_sync_replaces_only_managed_section_and_preserves_user_content() {
    let dir = TempDir::new().expect("tempdir");
    let path = dir.path().join("AGENTS.md");
    fs::write(
        &path,
        format!("head\n\n{MANAGED_BEGIN}\nold\n{MANAGED_END}\n\ntail\n"),
    )
    .expect("agents");

    sync_impl(&dir.path().to_string_lossy()).expect("sync");

    let updated = fs::read_to_string(path).expect("agents");
    assert!(updated.starts_with("head\n\n"));
    assert!(updated.ends_with("tail\n"));
    assert!(updated.contains("## Code quality guidance for AI agents"));
    assert!(!updated.contains("\nold\n"));
}

#[test]
fn agents_sync_is_idempotent() {
    let dir = TempDir::new().expect("tempdir");
    sync_impl(&dir.path().to_string_lossy()).expect("first sync");
    let once = fs::read_to_string(dir.path().join("AGENTS.md")).expect("agents");
    sync_impl(&dir.path().to_string_lossy()).expect("second sync");
    let twice = fs::read_to_string(dir.path().join("AGENTS.md")).expect("agents");

    assert_eq!(once, twice);
}

#[test]
fn upsert_managed_block_appends_when_missing() {
    let existing = "# Repository Rules\n\nKeep this text.\n";
    let updated = upsert_managed_block(existing, &managed_block());
    assert!(updated.contains("Keep this text."));
    assert!(updated.contains(MANAGED_BEGIN));
    assert!(updated.contains(MANAGED_END));
}

#[test]
fn serialized_json_is_schema_v3_and_matches_persisted_artifact() {
    let dir = TempDir::new().expect("tempdir");
    fs::create_dir_all(dir.path().join(".ayni/last")).expect("artifact directory");
    let artifact = RunArtifact::new(
        RunArtifactMetadata {
            generated_at: String::from("2026-07-12T00:00:00Z"),
            ayni_version: String::from("0.4.2"),
            invocation: InvocationContext {
                command: String::from("check"),
                languages: vec![Language::Rust],
                scope: None,
            },
            output: OutputContext {
                format: String::from("json"),
                destination: String::from("stdout"),
            },
            config_path: String::from("./.ayni.toml"),
            repository_root: String::from("."),
        },
        ayni_core::RunCompletion::complete(ayni_core::CompletionScope::Repository, 1),
        vec![test_row(true, 1, 0)],
    );
    let serialized = serialize_artifact(&artifact).expect("serialize artifact");
    persist_artifact_at(dir.path(), SIGNALS_ARTIFACT, &serialized).expect("persist artifact");

    let value: serde_json::Value = serde_json::from_str(&serialized).expect("valid json");
    assert_eq!(value["schema_version"], AYNI_SIGNAL_SCHEMA_VERSION);
    assert_eq!(value["generated_at"], "2026-07-12T00:00:00Z");
    assert_eq!(value["output"]["format"], "json");
    assert!(value.get("aggregate").is_some());
    assert!(value.get("applied_thresholds").is_some());
    assert_eq!(
        fs::read_to_string(dir.path().join(".ayni/last/signals.json")).expect("artifact"),
        serialized
    );
}

#[test]
fn kotlin_analyze_targets_are_built_when_enabled() {
    let dir = TempDir::new().expect("tempdir");
    fs::write(dir.path().join("build.gradle.kts"), "plugins {}\n").expect("gradle build");
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
enabled = ["kotlin"]

[kotlin]
roots = ["."]

[kotlin.size]
"**/*.kt" = { warn = 400, fail = 800 }
"#,
    )
    .expect("policy");

    let planning = super::build_analyze_targets(
        dir.path(),
        &policy,
        None,
        None,
        Some(Language::Kotlin),
        false,
    )
    .expect("targets");

    assert_eq!(planning.targets.len(), 1);
    assert_eq!(planning.targets[0].language, Language::Kotlin);
    assert_eq!(planning.targets[0].run_context.execution.runner, "gradle");
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

    let planning = super::build_analyze_targets(
        dir.path(),
        &policy,
        None,
        None,
        Some(Language::Python),
        false,
    )
    .expect("targets");
    assert_eq!(planning.targets.len(), 1);
    assert_eq!(planning.targets[0].language, Language::Python);
    assert_eq!(planning.targets[0].run_context.execution.runner, "python");
    assert_eq!(
        planning.targets[0].run_context.execution.kind,
        "direct_root"
    );
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
    }
}
