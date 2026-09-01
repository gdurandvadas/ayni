use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;

fn ayni() -> Command {
    Command::new(env!("CARGO_BIN_EXE_ayni"))
}

struct Fixture {
    _tempdir: TempDir,
    root: PathBuf,
    config: PathBuf,
}

impl Fixture {
    fn new(roots: &[&str], command_succeeds: bool) -> Self {
        let tempdir = TempDir::new().expect("tempdir");
        let root = tempdir.path().to_path_buf();
        let command = write_fixture_command(&root, command_succeeds);
        let roots = roots
            .iter()
            .map(|root| format!("\"{root}\""))
            .collect::<Vec<_>>()
            .join(", ");
        let config = root.join(".ayni.toml");
        fs::write(
            &config,
            format!(
                "[checks]\ntest = true\ncoverage = false\nsize = false\ncomplexity = false\ndeps = false\nmutation = false\n\n[languages]\nenabled = [\"rust\"]\n\n[rust]\nroots = [{roots}]\n\n[rust.tooling.test]\ncommand = \"{}\"\nargs = [\"fixture\"]\n",
                toml_string(&command)
            ),
        )
        .expect("policy");
        Self {
            _tempdir: tempdir,
            root,
            config,
        }
    }

    fn add_rust_root(&self, relative: &str) {
        let root = self.root.join(relative);
        fs::create_dir_all(&root).expect("root directory");
        fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .expect("Cargo manifest");
    }

    fn run(&self, args: &[&str]) -> Output {
        ayni()
            .args(args)
            .args(["--config", self.config.to_str().expect("config path")])
            .output()
            .expect("launch ayni")
    }

    fn artifact(&self, relative: &str) -> Value {
        serde_json::from_str(
            &fs::read_to_string(self.root.join(relative)).expect("persisted artifact"),
        )
        .expect("artifact JSON")
    }
}

fn toml_string(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}

#[cfg(unix)]
fn write_fixture_command(root: &Path, succeeds: bool) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let path = root.join("fixture-command");
    let body = if succeeds {
        "#!/bin/sh\nprintf '%s\\n' 'test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out'\n"
    } else {
        "#!/bin/sh\nprintf '%s\\n' 'fixture command failed' >&2\nexit 23\n"
    };
    fs::write(&path, body).expect("fixture command");
    let mut permissions = fs::metadata(&path).expect("metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&path, permissions).expect("executable command");
    path
}

#[cfg(windows)]
fn write_fixture_command(root: &Path, succeeds: bool) -> PathBuf {
    let path = root.join("fixture-command.cmd");
    let body = if succeeds {
        "@echo off\r\necho test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\r\n"
    } else {
        "@echo off\r\necho fixture command failed 1>&2\r\nexit /b 23\r\n"
    };
    fs::write(&path, body).expect("fixture command");
    path
}

#[test]
fn completion_keeps_undetected_configured_roots_and_replaces_stale_analyze_evidence() {
    let fixture = Fixture::new(&["good", "missing"], true);
    fixture.add_rust_root("good");
    let artifact_path = fixture.root.join(".ayni/last/signals.json");
    fs::create_dir_all(artifact_path.parent().expect("artifact parent")).expect("artifact dir");
    fs::write(&artifact_path, "stale-success\n").expect("stale artifact");

    let output = fixture.run(&["check", "--host", "--output", "json"]);
    assert_eq!(output.status.code(), Some(4));
    let artifact = fixture.artifact(".ayni/last/signals.json");
    assert_eq!(artifact["completion"]["scope"], "repository");
    assert_eq!(artifact["completion"]["state"], "incomplete");
    assert_eq!(artifact["completion"]["expected_targets"], 2);
    assert_eq!(artifact["completion"]["detected_targets"], 1);
    assert_eq!(artifact["completion"]["completed_targets"], 1);
    assert_eq!(artifact["completion"]["skipped_targets"], 1);
    assert_eq!(
        artifact["completion"]["issues"][0]["configured_root"],
        "missing"
    );
    assert_eq!(artifact["completion"]["issues"][0]["stage"], "detection");
    assert_eq!(artifact["rows"].as_array().expect("rows").len(), 1);
    assert_eq!(artifact["aggregate"]["status"], "fail");
    assert_ne!(
        fs::read_to_string(artifact_path).expect("artifact"),
        "stale-success\n"
    );
}

#[test]
fn completion_counts_failed_rows_as_completed_targets() {
    let fixture = Fixture::new(&["good"], false);
    fixture.add_rust_root("good");

    let output = fixture.run(&["check", "--host", "--output", "json"]);
    assert_eq!(output.status.code(), Some(4));
    let artifact = fixture.artifact(".ayni/last/signals.json");
    assert_eq!(artifact["completion"]["state"], "complete");
    assert_eq!(artifact["completion"]["completed_targets"], 1);
    assert_eq!(artifact["completion"]["skipped_targets"], 0);
    assert_eq!(artifact["rows"][0]["pass"], false);
    assert_eq!(artifact["rows"][0]["result"]["failure"]["exit_code"], 23);
}

#[test]
fn missing_expected_signal_row() {
    let fixture = Fixture::new(&["good"], true);
    fixture.add_rust_root("good");

    let output = fixture.run(&["check", "--host", "--output", "json"]);
    assert!(output.status.success(), "{:?}", output.stderr);
    let mut artifact = fixture.artifact(".ayni/last/signals.json");
    artifact["rows"] = serde_json::json!([]);
    artifact["applied_thresholds"] = serde_json::json!([]);
    artifact["offender_summaries"] = serde_json::json!([]);
    artifact["aggregate"] = serde_json::json!({
        "status": "fail",
        "total_rows": 0,
        "passing_rows": 0,
        "failing_rows": 0,
        "warning_offenders": 0,
        "failing_offenders": 0
    });

    let error = serde_json::from_value::<ayni_core::RunArtifact>(artifact)
        .expect_err("complete evidence cannot omit its expected signal row");
    assert!(error.to_string().contains("must contain rows"), "{error}");
}

#[test]
fn completion_verify_failure_replaces_only_requested_scope_artifact() {
    let fixture = Fixture::new(&["missing"], true);
    let analyze_path = fixture.root.join(".ayni/last/signals.json");
    let verify_path = fixture.root.join(".ayni/verify/last/signals.json");
    fs::create_dir_all(analyze_path.parent().expect("analyze parent")).expect("analyze dir");
    fs::create_dir_all(verify_path.parent().expect("verify parent")).expect("verify dir");
    fs::write(&analyze_path, "repository-evidence\n").expect("analyze evidence");
    fs::write(&verify_path, "stale-requested-success\n").expect("verify evidence");

    let output = fixture.run(&[
        "verify",
        "test",
        "--language",
        "rust",
        "--host",
        "--output",
        "json",
    ]);
    assert_eq!(output.status.code(), Some(4));
    let artifact = fixture.artifact(".ayni/verify/last/signals.json");
    assert_eq!(artifact["completion"]["scope"], "requested");
    assert_eq!(artifact["completion"]["state"], "incomplete");
    assert_eq!(artifact["completion"]["expected_targets"], 1);
    assert_eq!(artifact["completion"]["detected_targets"], 0);
    assert_eq!(artifact["completion"]["completed_targets"], 0);
    assert_eq!(artifact["completion"]["issues"][0]["stage"], "detection");
    assert_eq!(
        fs::read_to_string(analyze_path).expect("analyze evidence"),
        "repository-evidence\n"
    );
}

#[test]
fn adapter_configuration_error_is_incomplete_and_does_not_synthesize_a_row() {
    let fixture = Fixture::new(&["good"], true);
    fixture.add_rust_root("good");
    fs::write(
        &fixture.config,
        r#"[checks]
test = false
coverage = false
size = false
complexity = true
deps = false
mutation = false

[languages]
enabled = ["rust"]

[rust]
roots = ["good"]
"#,
    )
    .expect("policy");

    let output = fixture.run(&["check", "--host", "--output", "json"]);
    assert_eq!(output.status.code(), Some(4));

    let artifact = fixture.artifact(".ayni/last/signals.json");
    assert_eq!(artifact["completion"]["state"], "incomplete");
    assert_eq!(artifact["completion"]["completed_targets"], 0);
    assert_eq!(artifact["completion"]["skipped_targets"], 1);
    assert_eq!(artifact["completion"]["issues"][0]["stage"], "collection");
    assert!(
        artifact["completion"]["issues"][0]["message"]
            .as_str()
            .expect("completion message")
            .contains("missing expected complexity row")
    );
    assert!(artifact["rows"].as_array().expect("rows").is_empty());
    assert_eq!(artifact["aggregate"]["status"], "fail");
}

#[test]
fn invalid_contract_removes_stale_repository_evidence_before_target_planning() {
    let fixture = Fixture::new(&["good"], true);
    fixture.add_rust_root("good");
    let artifact_path = fixture.root.join(".ayni/last/signals.json");
    fs::create_dir_all(artifact_path.parent().expect("artifact parent")).expect("artifact dir");
    fs::write(&artifact_path, "stale-success\n").expect("stale artifact");
    fs::write(
        &fixture.config,
        r#"[checks]
coverage = true

[languages]
enabled = ["rust"]

[rust.coverage]
line_percent = { warn = 101, fail = 70 }
"#,
    )
    .expect("invalid policy");

    let output = fixture.run(&["check", "--host", "--output", "json"]);
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("rust.coverage.line_percent.warn must be finite and between 0 and 100"),
        "{stderr}"
    );
    assert!(
        !artifact_path.exists(),
        "invalid contracts must not leave stale evidence as current"
    );
}

#[cfg(unix)]
#[test]
fn configured_root_escape_is_rejected_by_analyze_before_artifact_writes() {
    use std::os::unix::fs::symlink;

    let fixture = Fixture::new(&["escape-link"], true);
    let outside_dir = TempDir::new().expect("outside tempdir");
    let outside = outside_dir.path();
    fs::write(
        outside.join("Cargo.toml"),
        "[package]\nname = \"outside\"\nversion = \"0.1.0\"\n",
    )
    .expect("outside manifest");
    symlink(outside, fixture.root.join("escape-link")).expect("escape link");

    let output = fixture.run(&["check", "--host", "--output", "json"]);
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("configured root 'escape-link'"), "{stderr}");
    assert!(stderr.contains("repository containment"), "{stderr}");
    assert!(!fixture.root.join(".ayni/last/signals.json").exists());
}
