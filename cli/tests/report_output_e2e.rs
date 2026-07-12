use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;

fn ayni() -> Command {
    Command::new(env!("CARGO_BIN_EXE_ayni"))
}

fn run_analyze(fixture: &Fixture, args: &[&str]) -> Output {
    ayni()
        .args([
            "analyze",
            "--config",
            fixture.config.to_str().expect("config path"),
        ])
        .args(args)
        .output()
        .expect("launch ayni binary")
}

struct Fixture {
    _tempdir: TempDir,
    root: PathBuf,
    config: PathBuf,
    command: PathBuf,
}

impl Fixture {
    fn new(test_succeeds: bool, include_size_warning: bool) -> Self {
        let tempdir = TempDir::new().expect("tempdir");
        let root = tempdir.path().to_path_buf();
        fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .expect("Cargo manifest");
        let command = write_fixture_command(&root, test_succeeds);
        if include_size_warning {
            fs::write(root.join("oversized.rs"), "line\nline\n").expect("oversized source");
        }
        let config = root.join(".ayni.toml");
        fs::write(&config, fixture_config(&command, include_size_warning)).expect("policy config");
        Self {
            _tempdir: tempdir,
            root,
            config,
            command,
        }
    }

    fn clear_artifact(&self) {
        let artifacts = self.root.join(".ayni");
        if artifacts.exists() {
            fs::remove_dir_all(artifacts).expect("remove prior artifact");
        }
    }

    fn persisted_artifact(&self) -> String {
        fs::read_to_string(self.root.join(".ayni/last/signals.json")).expect("persisted artifact")
    }
}

fn fixture_config(command: &Path, include_size_warning: bool) -> String {
    let size = if include_size_warning {
        "size = true"
    } else {
        "size = false"
    };
    let size_policy = if include_size_warning {
        "\n[rust.size]\n\"*.rs\" = { warn = 1, fail = 10 }\n"
    } else {
        ""
    };
    format!(
        "[checks]\ntest = true\ncoverage = false\n{size}\ncomplexity = false\ndeps = false\nmutation = false\n\n[languages]\nenabled = [\"rust\"]\n\n[rust.tooling.test]\ncommand = \"{}\"\nargs = [\"fixture\"]\n{size_policy}",
        toml_string(command),
    )
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
        "#!/bin/sh\nprintf '%s\\n' 'forced local collector failure' >&2\nexit 17\n"
    };
    fs::write(&path, body).expect("fixture command");
    let mut permissions = fs::metadata(&path).expect("command metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&path, permissions).expect("make fixture command executable");
    path
}

#[cfg(windows)]
fn write_fixture_command(root: &Path, succeeds: bool) -> PathBuf {
    let path = root.join("fixture-command.cmd");
    let body = if succeeds {
        "@echo off\r\necho test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\r\n"
    } else {
        "@echo off\r\necho forced local collector failure 1>&2\r\nexit /b 17\r\n"
    };
    fs::write(&path, body).expect("fixture command");
    path
}

#[test]
fn json_selectors_emit_only_json_and_match_persisted_artifacts() {
    let fixture = Fixture::new(true, false);
    let mut outputs = Vec::new();

    for args in [["--json"].as_slice(), ["--output", "json"].as_slice()] {
        fixture.clear_artifact();
        let output = run_analyze(&fixture, args);
        assert!(
            output.status.success(),
            "stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8(output.stdout).expect("UTF-8 JSON stdout");
        let artifact: Value = serde_json::from_str(&stdout).expect("schema-v2 JSON stdout");
        assert_eq!(artifact["schema_version"], "0.2.0");
        assert_eq!(artifact["output"]["format"], "json");
        assert!(!stdout.contains("running language="));
        assert!(!stdout.contains("command failure"));
        assert!(String::from_utf8_lossy(&output.stderr).contains("running language=rust"));

        let persisted = fixture.persisted_artifact();
        assert_eq!(stdout, persisted, "stdout must be the persisted artifact");
        outputs.push(artifact);
    }

    let mut short_selector = outputs.remove(0);
    let mut output_selector = outputs.remove(0);
    short_selector
        .as_object_mut()
        .expect("JSON object")
        .remove("generated_at");
    output_selector
        .as_object_mut()
        .expect("JSON object")
        .remove("generated_at");
    assert_eq!(
        short_selector, output_selector,
        "selectors have equivalent stable semantics"
    );
}

#[test]
fn conflicting_json_and_markdown_selectors_fail_before_analysis() {
    let output = ayni()
        .args(["analyze", "--json", "--output", "md"])
        .output()
        .expect("launch ayni binary");

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).is_empty());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("--json cannot be combined with --output md; use --output json or --json")
    );
}

#[test]
fn markdown_reports_real_local_command_failures_to_stdout_and_stderr() {
    let fixture = Fixture::new(false, false);
    let output = run_analyze(&fixture, &["--output", "md"]);
    assert!(!output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("UTF-8 Markdown stdout");
    let stderr = String::from_utf8(output.stderr).expect("UTF-8 stderr");
    let command = format!("{} fixture", fixture.command.display());
    let cwd = fixture.root.display().to_string();
    assert!(
        stdout.contains("## Failures"),
        "Markdown missing Failures heading: {stdout}"
    );
    for text in [
        "repo_code_issue",
        "command_error",
        command.as_str(),
        cwd.as_str(),
        "17",
        "forced local collector failure",
    ] {
        assert!(stdout.contains(text), "Markdown missing {text:?}: {stdout}");
        assert!(stderr.contains(text), "stderr missing {text:?}: {stderr}");
    }
    assert!(stderr.contains("command failure kind=Test"));
}

#[test]
fn successful_markdown_omits_failures_and_keeps_offenders() {
    let fixture = Fixture::new(true, true);
    let output = run_analyze(&fixture, &["--output", "md"]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("UTF-8 Markdown stdout");
    let stderr = String::from_utf8(output.stderr).expect("UTF-8 stderr");
    assert!(stdout.contains("<summary>Offenders</summary>"));
    assert!(stdout.contains("**WARN** `oversized.rs` lines=2 warn=1 fail=10"));
    assert!(!stdout.contains("## Failures"));
    assert!(!stderr.contains("command failure"));
}
