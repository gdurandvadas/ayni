use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;

fn ayni() -> Command {
    Command::new(env!("CARGO_BIN_EXE_ayni"))
}

struct RustFixture {
    _tempdir: TempDir,
    root: PathBuf,
    config: PathBuf,
}

impl RustFixture {
    fn new(checks: &str, extra_policy: &str) -> Self {
        let tempdir = TempDir::new().expect("tempdir");
        let root = tempdir.path().to_path_buf();
        fs::create_dir(root.join("src")).expect("src");
        fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .expect("manifest");
        fs::write(
            root.join("src/lib.rs"),
            "pub fn value() -> u8 {\n    1\n}\n",
        )
        .expect("source");
        let config = root.join(".ayni.toml");
        fs::write(
            &config,
            format!(
                "[checks]\n{checks}\n\n[languages]\nenabled = [\"rust\"]\n\n[rust]\nroots = [\".\"]\n{extra_policy}"
            ),
        )
        .expect("policy");
        Self {
            _tempdir: tempdir,
            root,
            config,
        }
    }

    fn run(&self, args: &[&str]) -> Output {
        ayni()
            .args(["verify"])
            .args(args)
            .args(["--host", "--config", self.config.to_str().expect("config")])
            .output()
            .expect("launch ayni")
    }

    fn artifact(&self) -> Value {
        serde_json::from_str(
            &fs::read_to_string(self.root.join(".ayni/verify/last/signals.json"))
                .expect("verification artifact"),
        )
        .expect("artifact JSON")
    }
}

fn all_checks(enabled: &str) -> String {
    ["test", "coverage", "size", "complexity", "deps", "mutation"]
        .into_iter()
        .map(|kind| format!("{kind} = {}", kind == enabled))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn emitted_multi_root_command_is_reproducible() {
    let tempdir = TempDir::new().expect("tempdir");
    let root = tempdir.path();
    for configured_root in ["services/one", "services/two"] {
        let target = root.join(configured_root);
        fs::create_dir_all(&target).expect("target root");
        fs::write(
            target.join("Cargo.toml"),
            "[package]\nname = \"same-name\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .expect("manifest");
        fs::write(target.join("oversized.rs"), "one\ntwo\nthree\n").expect("source");
    }
    let config = root.join("focused.toml");
    fs::write(
        &config,
        format!(
            "[checks]\n{}\n\n[languages]\nenabled = [\"rust\"]\n\n[rust]\nroots = [\"services/one\", \"services/two\"]\n\n[rust.size]\n\"*.rs\" = {{ warn = 1, fail = 2 }}\n",
            all_checks("size")
        ),
    )
    .expect("policy");

    let analyze = ayni()
        .args([
            "check",
            "--host",
            "--config",
            config.to_str().expect("config"),
            "--output",
            "json",
        ])
        .output()
        .expect("check");
    assert!(!analyze.status.success(), "size findings must fail");
    let artifact: Value = serde_json::from_slice(&analyze.stdout).expect("artifact");
    let commands = artifact["rows"]
        .as_array()
        .expect("rows")
        .iter()
        .map(|row| {
            row["offenders"]["items"][0]["verification"]["command"]
                .as_str()
                .expect("command")
        })
        .collect::<Vec<_>>();
    assert_eq!(commands.len(), 2);
    assert!(commands[0].contains("--root 'services/one'"));
    assert!(commands[1].contains("--root 'services/two'"));

    // Execute the first emitted command's exact selector values. A finding is
    // expected, but requested completion must contain only its originating root.
    let reproduced = ayni()
        .args([
            "verify",
            "size",
            "--host",
            "--config",
            config.to_str().expect("config"),
            "--language",
            "rust",
            "--root",
            "services/one",
            "--file",
            "services/one/oversized.rs",
            "--output",
            "json",
        ])
        .output()
        .expect("reproduce finding command");
    assert_eq!(
        reproduced.status.code(),
        Some(1),
        "finding remains a quality failure"
    );
    let reproduced: Value = serde_json::from_slice(&reproduced.stdout).expect("verify artifact");
    assert_eq!(reproduced["completion"]["expected_targets"], 1);
    assert_eq!(reproduced["completion"]["completed_targets"], 1);
    assert_eq!(reproduced["rows"].as_array().expect("rows").len(), 1);
    assert_eq!(reproduced["rows"][0]["scope"]["path"], "services/one");
}

#[test]
fn size_file_verification_is_exact_requested_evidence_and_preserves_analyze_artifact() {
    let fixture = RustFixture::new(
        &all_checks("size"),
        "\n[rust.size]\n\"*.rs\" = { warn = 100, fail = 200 }\n",
    );
    let analyze = fixture.root.join(".ayni/last/signals.json");
    fs::create_dir_all(analyze.parent().expect("parent")).expect("analyze dir");
    fs::write(&analyze, "repository evidence\n").expect("analyze evidence");

    let output = fixture.run(&["size", "--file", "src/lib.rs", "--output", "json"]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("UTF-8 stdout");
    let artifact: Value = serde_json::from_str(&stdout).expect("JSON-only stdout");
    assert_eq!(artifact, fixture.artifact());
    assert_eq!(artifact["completion"]["scope"], "requested");
    assert_eq!(artifact["completion"]["state"], "complete");
    assert_eq!(artifact["completion"]["expected_targets"], 1);
    assert_eq!(artifact["rows"].as_array().expect("rows").len(), 1);
    assert_eq!(artifact["rows"][0]["kind"], "size");
    assert_eq!(artifact["rows"][0]["scope"]["file"], "src/lib.rs");
    assert!(artifact["rows"][0].get("delta_vs_previous").is_none());
    assert_eq!(
        fs::read_to_string(analyze).expect("analyze evidence"),
        "repository evidence\n"
    );
}

#[test]
fn policy_and_selector_validation_happen_before_tool_execution() {
    let fixture = RustFixture::new(&all_checks("test"), "");
    let marker = fixture.root.join("tool-ran");
    let command = write_marker_command(&fixture.root, &marker);
    fs::write(
        &fixture.config,
        format!(
            "[checks]\n{}\n\n[languages]\nenabled = [\"rust\"]\n\n[rust.tooling.test]\ncommand = \"{}\"\n",
            all_checks("test"),
            toml_string(&command)
        ),
    )
    .expect("policy override");

    for args in [
        ["test", "--file", "src/lib.rs"].as_slice(),
        ["test", "--file", "src/lib.rs", "--package", "fixture"].as_slice(),
        ["size", "--language", "rust"].as_slice(),
    ] {
        let output = fixture.run(args);
        assert_eq!(
            output.status.code(),
            Some(2),
            "{args:?} must be rejected as invalid input"
        );
    }
    let invalid_root = fixture.run(&[
        "test",
        "--language",
        "rust",
        "--root",
        "missing",
        "--file",
        "src/lib.rs",
    ]);
    assert_eq!(invalid_root.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&invalid_root.stderr).contains("not a normalized configured root"),
        "root validation must precede adapter rejection of test --file"
    );
    assert!(!marker.exists(), "validation must precede tool invocation");
}

#[test]
fn package_and_dependency_forms_select_one_rust_target() {
    let fixture = RustFixture::new(&all_checks("deps"), "");
    let output = fixture.run(&[
        "deps",
        "--language",
        "rust",
        "--package",
        "fixture",
        "--output",
        "json",
    ]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let artifact = fixture.artifact();
    assert_eq!(artifact["rows"].as_array().expect("rows").len(), 1);
    assert_eq!(artifact["rows"][0]["kind"], "deps");
    assert_eq!(artifact["rows"][0]["scope"]["package"], "fixture");
}

#[test]
fn ambiguous_language_and_unsafe_file_requests_fail_without_artifacts() {
    let fixture = RustFixture::new(&all_checks("size"), "");
    fs::write(fixture.root.join("package.json"), "{}\n").expect("node manifest");
    fs::write(
        &fixture.config,
        format!(
            "[checks]\n{}\n\n[languages]\nenabled = [\"rust\", \"node\"]\n",
            all_checks("size")
        ),
    )
    .expect("polyglot policy");

    let ambiguous = fixture.run(&["size"]);
    assert_eq!(ambiguous.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&ambiguous.stderr).contains("--language is required"));
    for file in ["../outside.rs", "/tmp/outside.rs"] {
        let output = fixture.run(&["size", "--file", file]);
        assert_eq!(output.status.code(), Some(2));
    }
    assert!(!fixture.root.join(".ayni/verify/last/signals.json").exists());
}

#[cfg(unix)]
#[test]
fn configured_root_escape_is_rejected_by_verify_before_artifact_writes() {
    use std::os::unix::fs::symlink;

    let fixture = RustFixture::new(&all_checks("test"), "");
    let outside_dir = TempDir::new().expect("outside tempdir");
    let outside = outside_dir.path();
    fs::write(
        outside.join("Cargo.toml"),
        "[package]\nname = \"outside\"\nversion = \"0.1.0\"\n",
    )
    .expect("outside manifest");
    symlink(outside, fixture.root.join("escape-link")).expect("escape link");
    fs::write(
        &fixture.config,
        format!(
            "[checks]\n{}\n\n[languages]\nenabled = [\"rust\"]\n\n[rust]\nroots = [\"escape-link\"]\n",
            all_checks("test")
        ),
    )
    .expect("policy");

    let output = fixture.run(&["test", "--language", "rust"]);
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("configured root 'escape-link'"), "{stderr}");
    assert!(stderr.contains("repository containment"), "{stderr}");
    assert!(!fixture.root.join(".ayni/verify").exists());
}

fn toml_string(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}

#[cfg(unix)]
fn write_marker_command(root: &Path, marker: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let path = root.join("marker-command");
    fs::write(
        &path,
        format!("#!/bin/sh\nprintf ran > '{}'\n", marker.display()),
    )
    .expect("command");
    let mut permissions = fs::metadata(&path).expect("metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&path, permissions).expect("permissions");
    path
}

#[cfg(windows)]
fn write_marker_command(root: &Path, marker: &Path) -> PathBuf {
    let path = root.join("marker-command.cmd");
    fs::write(&path, format!("@echo ran > \"{}\"\r\n", marker.display())).expect("command");
    path
}
