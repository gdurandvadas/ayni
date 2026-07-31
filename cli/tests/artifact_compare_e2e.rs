use ayni_core::{
    Budget, CompletionScope, Offenders, RunArtifact, RunCompletion, Scope, SignalKind,
    SignalResult, SignalRow, TestResult,
};
use std::fs;
use std::process::Command;
use tempfile::TempDir;

fn artifact(pass: bool, passed: u64, failed: u64) -> String {
    let artifact = RunArtifact {
        completion: RunCompletion::complete(CompletionScope::Repository, 1),
        rows: vec![SignalRow {
            kind: SignalKind::Test,
            language: ayni_core::Language::Rust,
            scope: Scope {
                workspace_root: String::from("."),
                path: None,
                package: None,
                file: None,
            },
            pass,
            result: SignalResult::Test(TestResult {
                total_tests: passed + failed,
                passed,
                failed,
                duration_ms: None,
                runner: String::from("fixture"),
                failure: None,
            }),
            budget: Budget::Test(serde_json::json!({})),
            offenders: Offenders::Test(Vec::new()),
        }],
        ..RunArtifact::default()
    };
    serde_json::to_string(&artifact).expect("serialize artifact")
}

#[test]
fn compare_reads_explicit_artifacts_and_emits_one_json_document() {
    let tempdir = TempDir::new().expect("tempdir");
    let baseline = tempdir.path().join("baseline.json");
    let candidate = tempdir.path().join("candidate.json");
    fs::write(&baseline, artifact(false, 8, 2)).expect("baseline");
    fs::write(&candidate, artifact(true, 10, 0)).expect("candidate");

    let output = Command::new(env!("CARGO_BIN_EXE_ayni"))
        .args([
            "artifact",
            "compare",
            "--baseline",
            baseline.to_str().expect("baseline path"),
            "--candidate",
            candidate.to_str().expect("candidate path"),
            "--output",
            "json",
        ])
        .output()
        .expect("launch ayni");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let comparison: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("JSON output");
    assert_eq!(comparison["comparison_schema_version"], "0.1.0");
    assert_eq!(comparison["matched"][0]["changed"], true);
    assert_eq!(comparison["matched"][0]["changes"]["pass"]["before"], false);
    assert_eq!(comparison["matched"][0]["changes"]["pass"]["after"], true);
    assert_eq!(comparison["added"], serde_json::json!([]));
}

#[test]
fn compare_rejects_invalid_input_without_json_stdout() {
    let tempdir = TempDir::new().expect("tempdir");
    let baseline = tempdir.path().join("baseline.json");
    let candidate = tempdir.path().join("candidate.json");
    fs::write(&baseline, "not JSON").expect("baseline");
    fs::write(&candidate, artifact(true, 1, 0)).expect("candidate");

    let output = Command::new(env!("CARGO_BIN_EXE_ayni"))
        .args([
            "artifact",
            "compare",
            "--baseline",
            baseline.to_str().expect("baseline path"),
            "--candidate",
            candidate.to_str().expect("candidate path"),
            "--output",
            "json",
        ])
        .output()
        .expect("launch ayni");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("could not parse baseline artifact"));
}
