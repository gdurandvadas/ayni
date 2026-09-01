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

#[cfg(unix)]
struct NodeStartupFailureFixture {
    _tempdir: TempDir,
    root: PathBuf,
}

#[cfg(unix)]
impl NodeStartupFailureFixture {
    fn new() -> Self {
        use std::os::unix::fs::PermissionsExt;

        let tempdir = TempDir::new().expect("tempdir");
        let root = tempdir.path().to_path_buf();
        fs::write(root.join("package.json"), r#"{"name":"node-fixture"}"#).expect("Node manifest");
        let runner = root.join("failing-vitest");
        fs::write(
            &runner,
            "#!/bin/sh\nprintf \"Error: Cannot find module '@sveltejs/vite-plugin-svelte'\\n\" >&2\nexit 1\n",
        )
        .expect("failing Vitest fixture");
        let mut permissions = fs::metadata(&runner)
            .expect("runner metadata")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&runner, permissions).expect("executable runner");
        fs::write(
            root.join(".ayni.toml"),
            format!(
                "[checks]\n{}\n\n[languages]\nenabled = [\"node\"]\n\n[node]\nroots = [\".\"]\n\n[node.tooling.test]\ncommand = \"{}\"\nargs = [\"run\"]\n",
                all_checks("test"),
                toml_string(&runner)
            ),
        )
        .expect("policy");
        Self {
            _tempdir: tempdir,
            root,
        }
    }

    fn run(&self, operation: &[&str], selectors: &[&str]) -> Output {
        ayni()
            .current_dir(&self.root)
            .args(operation)
            .args(["--host", "--config", "./.ayni.toml", "--output", "json"])
            .args(selectors)
            .output()
            .expect("run Node startup failure fixture")
    }
}

#[cfg(unix)]
fn assert_valid_node_startup_failure(output: &Output, persisted_path: &Path) {
    assert_eq!(
        output.status.code(),
        Some(4),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!String::from_utf8_lossy(&output.stderr).contains("failed to serialize artifact"));
    let artifact: Value = serde_json::from_slice(&output.stdout).expect("valid failed artifact");
    assert_eq!(artifact["completion"]["state"], "complete");
    assert_eq!(artifact["rows"][0]["result"]["total_tests"], 0);
    assert_eq!(artifact["rows"][0]["result"]["passed"], 0);
    assert_eq!(artifact["rows"][0]["result"]["failed"], 0);
    assert_eq!(
        artifact["rows"][0]["result"]["failure"]["classification"],
        "import_error"
    );
    assert!(
        artifact["rows"][0]["result"]["failure"]["message"]
            .as_str()
            .expect("failure message")
            .contains("@sveltejs/vite-plugin-svelte")
    );
    let persisted: Value =
        serde_json::from_slice(&fs::read(persisted_path).expect("persisted artifact"))
            .expect("persisted artifact JSON");
    assert_eq!(persisted, artifact);
}

#[cfg(unix)]
#[test]
fn node_verify_startup_failure_writes_a_valid_failed_artifact() {
    let fixture = NodeStartupFailureFixture::new();
    let output = fixture.run(&["verify", "test"], &["--language", "node"]);

    assert_valid_node_startup_failure(
        &output,
        &fixture.root.join(".ayni/verify/last/signals.json"),
    );
}

#[cfg(unix)]
#[test]
fn node_check_startup_failure_writes_a_valid_repository_artifact() {
    let fixture = NodeStartupFailureFixture::new();
    let output = fixture.run(&["check"], &[]);

    assert_valid_node_startup_failure(&output, &fixture.root.join(".ayni/last/signals.json"));
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
fn relative_config_preserves_nested_node_workspace_dependency_members() {
    let tempdir = TempDir::new().expect("tempdir");
    let root = tempdir.path();
    let frontend = root.join("frontend");
    fs::create_dir_all(frontend.join("apps/web")).expect("web package");
    fs::create_dir_all(frontend.join("packages/bff")).expect("bff package");
    fs::create_dir_all(frontend.join("packages/ui")).expect("ui package");
    fs::write(
        frontend.join("package.json"),
        r#"{"name":"@fixture/frontend","private":true,"workspaces":["apps/*","packages/*"]}"#,
    )
    .expect("workspace manifest");
    fs::write(
        frontend.join("apps/web/package.json"),
        r#"{"name":"@fixture/web","dependencies":{"@fixture/bff":"workspace:*","@fixture/ui":"workspace:*"}}"#,
    )
    .expect("web manifest");
    fs::write(
        frontend.join("packages/bff/package.json"),
        r#"{"name":"@fixture/bff"}"#,
    )
    .expect("bff manifest");
    fs::write(
        frontend.join("packages/ui/package.json"),
        r#"{"name":"@fixture/ui"}"#,
    )
    .expect("ui manifest");
    let config = root.join(".ayni.toml");
    fs::write(
        &config,
        format!(
            "[checks]\n{}\n\n[languages]\nenabled = [\"node\"]\n\n[node]\nroots = [\"frontend\"]\n\n[node.deps.forbidden]\n\"frontend/apps/web\" = [\"frontend/packages/bff\"]\n",
            all_checks("deps")
        ),
    )
    .expect("policy");

    let run = |config_path: &str| {
        ayni()
            .current_dir(root)
            .args([
                "verify",
                "deps",
                "--host",
                "--language",
                "node",
                "--config",
                config_path,
                "--output",
                "json",
            ])
            .output()
            .expect("verify node dependencies")
    };

    let relative = run("./.ayni.toml");
    assert_eq!(
        relative.status.code(),
        Some(1),
        "stderr: {}",
        String::from_utf8_lossy(&relative.stderr)
    );
    let relative: Value = serde_json::from_slice(&relative.stdout).expect("relative artifact");
    assert_eq!(relative["config_path"], "./.ayni.toml");
    assert_eq!(
        relative["repository_root"],
        root.canonicalize()
            .expect("canonical fixture")
            .to_string_lossy()
            .as_ref()
    );
    assert_eq!(relative["rows"][0]["result"]["crate_count"], 4);
    assert_eq!(relative["rows"][0]["result"]["edge_count"], 2);
    assert_eq!(relative["rows"][0]["result"]["violation_count"], 1);
    assert_eq!(relative["rows"][0]["pass"], false);

    let canonical_config = config.canonicalize().expect("canonical config");
    let absolute = run(canonical_config.to_str().expect("config path"));
    assert_eq!(absolute.status.code(), Some(1));
    let absolute: Value = serde_json::from_slice(&absolute.stdout).expect("absolute artifact");
    assert_eq!(
        absolute["config_path"],
        canonical_config.to_string_lossy().as_ref()
    );
    assert_eq!(relative["repository_root"], absolute["repository_root"]);
    assert_eq!(
        relative["rows"][0]["scope"]["workspace_root"],
        absolute["rows"][0]["scope"]["workspace_root"]
    );
    assert_eq!(relative["rows"][0]["result"], absolute["rows"][0]["result"]);
    assert_eq!(relative["rows"][0]["pass"], absolute["rows"][0]["pass"]);
    assert_eq!(
        relative["rows"][0]["offenders"]["items"][0]["id"],
        absolute["rows"][0]["offenders"]["items"][0]["id"]
    );

    let managed_lock_fingerprint = format!("sha256:{}", "1".repeat(64));
    let managed = ayni()
        .current_dir(root)
        .env(
            "AYNI_MANAGED_TARGET_ENVIRONMENTS",
            r#"{"node:frontend":{}}"#,
        )
        .env("AYNI_MANAGED_LOCK_FINGERPRINT", &managed_lock_fingerprint)
        .env(
            "AYNI_MANAGED_TOOL_VERSIONS",
            r#"[{"tool":"node:frontend:runtime:node","version":"24.14.0"}]"#,
        )
        .args([
            "verify",
            "deps",
            "--host",
            "--language",
            "node",
            "--config",
            "./.ayni.toml",
            "--output",
            "json",
        ])
        .output()
        .expect("managed inner Node dependency verification");
    assert_eq!(
        managed.status.code(),
        Some(1),
        "stderr: {}",
        String::from_utf8_lossy(&managed.stderr)
    );
    let managed: Value = serde_json::from_slice(&managed.stdout).expect("managed artifact");
    assert_eq!(managed["execution_mode"], "managed");
    assert_eq!(managed["config_path"], "./.ayni.toml");
    assert_eq!(
        managed["repository_root"],
        root.canonicalize()
            .expect("canonical fixture")
            .to_string_lossy()
            .as_ref()
    );
    assert_eq!(
        managed["environment_lock_fingerprint"],
        managed_lock_fingerprint
    );
    assert_eq!(managed["completion"]["scope"], "requested");
    assert_eq!(managed["completion"]["state"], "complete");
    assert_eq!(managed["rows"][0]["result"]["crate_count"], 4);
    assert_eq!(managed["rows"][0]["result"]["edge_count"], 2);
    assert_eq!(managed["rows"][0]["result"]["violation_count"], 1);
    assert_eq!(managed["rows"][0]["result"], relative["rows"][0]["result"]);

    let run_check = |config_path: &str| {
        ayni()
            .current_dir(root)
            .args([
                "check",
                "--host",
                "--config",
                config_path,
                "--output",
                "json",
            ])
            .output()
            .expect("check node dependencies")
    };
    let relative_check = run_check("./.ayni.toml");
    assert_eq!(relative_check.status.code(), Some(1));
    let relative_check: Value =
        serde_json::from_slice(&relative_check.stdout).expect("relative check artifact");
    assert_eq!(relative_check["completion"]["scope"], "repository");
    assert_eq!(relative_check["completion"]["state"], "complete");
    assert_eq!(
        relative_check["rows"][0]["result"],
        relative["rows"][0]["result"]
    );
    assert!(
        relative_check["rows"][0]["offenders"]["items"][0]["verification"]["command"]
            .as_str()
            .expect("verification command")
            .contains("--config './.ayni.toml'")
    );

    let absolute_check = run_check(canonical_config.to_str().expect("config path"));
    assert_eq!(absolute_check.status.code(), Some(1));
    let absolute_check: Value =
        serde_json::from_slice(&absolute_check.stdout).expect("absolute check artifact");
    assert_eq!(
        relative_check["rows"][0]["result"],
        absolute_check["rows"][0]["result"]
    );
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
    let stale_artifact = fixture.root.join(".ayni/verify/last/signals.json");
    fs::create_dir_all(stale_artifact.parent().expect("artifact parent"))
        .expect("artifact directory");
    fs::write(&stale_artifact, "stale-success\n").expect("stale artifact");

    let ambiguous = fixture.run(&["size"]);
    assert_eq!(ambiguous.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&ambiguous.stderr).contains("--language is required"));
    assert!(
        !stale_artifact.exists(),
        "a rejected request must not leave prior focused evidence current"
    );
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
