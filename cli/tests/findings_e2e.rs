//! Black-box coverage for schema-v3 finding artifacts and their displayed
//! verification commands.

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
    fn size() -> Self {
        Self::new(
            "size = true",
            "\n[rust.size]\n\"*.rs\" = { warn = 1, fail = 2 }\n",
            None,
        )
    }

    fn zero_tests() -> Self {
        let tempdir = TempDir::new().expect("tempdir");
        let root = tempdir.path().to_path_buf();
        fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .expect("manifest");
        let command = write_zero_test_command(&root);
        let config = root.join(".ayni.toml");
        fs::write(
            &config,
            format!(
                "[checks]\ntest = true\ncoverage = false\nsize = false\ncomplexity = false\ndeps = false\nmutation = false\n\n[languages]\nenabled = [\"rust\"]\n\n[rust.tooling.test]\ncommand = \"{}\"\n",
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

    fn new(check: &str, policy: &str, source: Option<&str>) -> Self {
        let tempdir = TempDir::new().expect("tempdir");
        let root = tempdir.path().to_path_buf();
        fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .expect("manifest");
        fs::write(
            root.join("oversized.rs"),
            source.unwrap_or("one\ntwo\nthree\n"),
        )
        .expect("source");
        let config = root.join(".ayni.toml");
        fs::write(
            &config,
            format!(
                "[checks]\ntest = false\ncoverage = false\n{check}\ncomplexity = false\ndeps = false\nmutation = false\n\n[languages]\nenabled = [\"rust\"]\n{policy}"
            ),
        )
        .expect("policy");
        Self {
            _tempdir: tempdir,
            root,
            config,
        }
    }

    fn analyze(&self, args: &[&str]) -> Output {
        ayni()
            .args([
                "check",
                "--host",
                "--config",
                self.config.to_str().expect("config"),
            ])
            .args(args)
            .output()
            .expect("launch ayni")
    }

    fn persisted(&self) -> String {
        fs::read_to_string(self.root.join(".ayni/last/signals.json")).expect("artifact")
    }
}

fn toml_string(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}

#[cfg(unix)]
fn write_zero_test_command(root: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let command = root.join("zero-tests");
    fs::write(
        &command,
        "#!/bin/sh\nprintf '%s\\n' 'test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out'\n",
    )
    .expect("command");
    let mut permissions = fs::metadata(&command).expect("metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&command, permissions).expect("executable command");
    command
}

#[cfg(windows)]
fn write_zero_test_command(root: &Path) -> PathBuf {
    let command = root.join("zero-tests.cmd");
    fs::write(
        &command,
        "@echo off\r\necho test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\r\n",
    )
    .expect("command");
    command
}

fn assert_public_finding(finding: &Value, command: &str) {
    let id = finding["id"].as_str().expect("finding id");
    assert!(id.starts_with("ayni:finding:v1:sha256:"));
    assert_eq!(id.len(), 87);
    assert_eq!(finding["verification"]["command"], command);
    assert!(finding["verification"].get("target").is_none());
    assert!(finding.get("offender").is_none());
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

#[test]
fn preserves_non_default_config_and_root() {
    let tempdir = TempDir::new().expect("tempdir");
    let root = tempdir.path();
    let configured_root = "crates/hostile root's $(safe)";
    let target = root.join(configured_root);
    fs::create_dir_all(&target).expect("target root");
    fs::write(
        target.join("Cargo.toml"),
        "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .expect("manifest");
    fs::write(target.join("oversized.rs"), "one\ntwo\nthree\n").expect("source");
    let config = root.join("policy it's $(safe).toml");
    fs::write(
        &config,
        format!(
            "[checks]\ntest = false\ncoverage = false\nsize = true\ncomplexity = false\ndeps = false\nmutation = false\n\n[languages]\nenabled = [\"rust\"]\n\n[rust]\nroots = [{}]\n\n[rust.size]\n\"*.rs\" = {{ warn = 1, fail = 2 }}\n",
            toml::Value::String(configured_root.to_string())
        ),
    )
    .expect("policy");

    let output = ayni()
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
    assert!(!output.status.success(), "size finding must fail");
    let artifact: Value = serde_json::from_slice(&output.stdout).expect("artifact");
    let command = artifact["rows"][0]["offenders"]["items"][0]["verification"]["command"]
        .as_str()
        .expect("command");
    assert_eq!(
        command,
        format!(
            "ayni verify size --config {} --language rust --root {} --host --file {}",
            shell_quote(&config.to_string_lossy()),
            shell_quote(configured_root),
            shell_quote(&format!("{configured_root}/oversized.rs")),
        )
    );
}

#[test]
fn size_finding_is_flat_and_identical_in_json_persistence_and_reports() {
    let fixture = Fixture::size();
    let command = format!(
        "ayni verify size --config '{}' --language rust --root '.' --host --file 'oversized.rs'",
        fixture.config.display()
    );

    let json = fixture.analyze(&["--output", "json"]);
    assert!(!json.status.success(), "size finding must fail the run");
    let stdout = String::from_utf8(json.stdout).expect("JSON stdout");
    assert_eq!(stdout, fixture.persisted());
    let artifact: Value = serde_json::from_str(&stdout).expect("schema-v3 JSON");
    let finding = &artifact["rows"][0]["offenders"]["items"][0];
    assert_public_finding(finding, &command);

    let terminal = fixture.analyze(&[]);
    assert!(!terminal.status.success());
    assert!(String::from_utf8_lossy(&terminal.stdout).contains(&command));

    let markdown = fixture.analyze(&["--output", "markdown"]);
    assert!(!markdown.status.success());
    assert!(String::from_utf8_lossy(&markdown.stdout).contains(&format!("- `{command}`")));
}

#[test]
fn synthetic_zero_test_finding_has_an_actionable_public_command() {
    let fixture = Fixture::zero_tests();
    let command = format!(
        "ayni verify test --config '{}' --language rust --root '.' --host",
        fixture.config.display()
    );
    let output = fixture.analyze(&["--output", "json"]);
    assert!(!output.status.success(), "zero tests must fail the run");
    let stdout = String::from_utf8(output.stdout).expect("JSON stdout");
    assert_eq!(stdout, fixture.persisted());
    let artifact: Value = serde_json::from_str(&stdout).expect("schema-v3 JSON");
    assert_eq!(artifact["rows"][0]["result"]["total_tests"], 0);
    assert_public_finding(&artifact["rows"][0]["offenders"]["items"][0], &command);
}
