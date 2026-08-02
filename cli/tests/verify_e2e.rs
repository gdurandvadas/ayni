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
            .args(["--config", self.config.to_str().expect("config")])
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
fn size_file_verification_is_exact_requested_evidence_and_preserves_analyze_artifact() {
    let fixture = RustFixture::new(
        &all_checks("size"),
        "\n[rust.size]\n\"*.rs\" = { warn = 100, fail = 200 }\n",
    );
    let analyze = fixture.root.join(".ayni/last/signals.json");
    fs::create_dir_all(analyze.parent().expect("parent")).expect("analyze dir");
    fs::write(&analyze, "repository evidence\n").expect("analyze evidence");

    let output = fixture.run(&["size", "--file", "src/lib.rs", "--json"]);
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
        assert!(!output.status.success(), "{args:?} unexpectedly succeeded");
    }
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
        "--json",
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
    assert!(!ambiguous.status.success());
    assert!(String::from_utf8_lossy(&ambiguous.stderr).contains("--language is required"));
    for file in ["../outside.rs", "/tmp/outside.rs"] {
        let output = fixture.run(&["size", "--file", file]);
        assert!(!output.status.success());
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
    assert!(!output.status.success());
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
