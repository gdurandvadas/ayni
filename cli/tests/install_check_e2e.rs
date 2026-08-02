use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;

#[derive(Debug, PartialEq, Eq)]
enum SnapshotEntry {
    Directory,
    File(Vec<u8>),
}

fn ayni() -> Command {
    Command::new(env!("CARGO_BIN_EXE_ayni"))
}

fn check(root: &Path, json: bool) -> Output {
    let mut command = ayni();
    command
        .args(["install", "--check", "--repo-root"])
        .arg(root);
    if json {
        command.args(["--output", "json"]);
    }
    command.output().expect("launch ayni")
}

fn check_with_path(root: &Path, checks: &str) -> Value {
    fs::write(root.join(".ayni.toml"), format!(
        "[checks]\ntest = {}\ncoverage = {}\nsize = false\ncomplexity = false\ndeps = false\nmutation = false\n\n[languages]\nenabled = [\"node\", \"python\"]\n\n[node]\nroots = [\".\"]\n\n[python]\nroots = [\".\"]\n", checks == "test", checks == "coverage"
    )).expect("policy");
    fs::write(root.join("package.json"), "{}\n").expect("node manifest");
    fs::write(
        root.join("pyproject.toml"),
        "[project]\nname = \"fixture\"\n",
    )
    .expect("python manifest");
    let empty_path = root.join("empty-path");
    if !empty_path.exists() {
        fs::create_dir(&empty_path).expect("empty PATH");
    }
    let mut command = ayni();
    let output = command
        .env("PATH", &empty_path)
        .args(["install", "--check", "--repo-root", ".", "--output", "json"])
        .current_dir(root)
        .output()
        .expect("launch ayni");
    assert!(!output.status.success());
    assert!(output.stderr.is_empty());
    serde_json::from_slice(&output.stdout).expect("readiness JSON")
}

fn snapshot(root: &Path) -> BTreeMap<PathBuf, SnapshotEntry> {
    fn visit(root: &Path, current: &Path, out: &mut BTreeMap<PathBuf, SnapshotEntry>) {
        let mut entries = fs::read_dir(current)
            .expect("read fixture directory")
            .map(|entry| entry.expect("directory entry").path())
            .collect::<Vec<_>>();
        entries.sort();
        for path in entries {
            let relative = path
                .strip_prefix(root)
                .expect("relative path")
                .to_path_buf();
            if path.is_dir() {
                out.insert(relative, SnapshotEntry::Directory);
                visit(root, &path, out);
            } else {
                out.insert(
                    relative,
                    SnapshotEntry::File(fs::read(&path).expect("read fixture file")),
                );
            }
        }
    }
    let mut files = BTreeMap::new();
    visit(root, root, &mut files);
    files
}

fn write_policy(root: &Path, configured_root: &str) {
    fs::write(
        root.join(".ayni.toml"),
        format!(
            "[checks]\ntest = false\ncoverage = false\nsize = true\ncomplexity = false\ndeps = false\nmutation = false\n\n[languages]\nenabled = [\"rust\"]\n\n[rust]\nroots = [\"{configured_root}\"]\n\n[rust.size]\n\"*.rs\" = {{ warn = 400, fail = 800 }}\n"
        ),
    )
    .expect("write policy");
}

#[test]
fn json_not_ready_is_byte_stable_and_never_mutates_paths() {
    let tempdir = TempDir::new().expect("tempdir");
    write_policy(tempdir.path(), "missing");
    fs::write(tempdir.path().join("sentinel"), b"unchanged\n").expect("sentinel");
    let before = snapshot(tempdir.path());

    let first = check(tempdir.path(), true);
    let second = check(tempdir.path(), true);

    assert!(!first.status.success());
    assert!(!second.status.success());
    assert!(first.stderr.is_empty());
    assert!(second.stderr.is_empty());
    assert_eq!(
        first.stdout, second.stdout,
        "JSON must be byte-for-byte stable"
    );
    assert_eq!(
        snapshot(tempdir.path()),
        before,
        "check mutated repository paths"
    );
    assert!(!tempdir.path().join(".ayni").exists());
    assert!(!tempdir.path().join(".gitignore").exists());

    let value: Value = serde_json::from_slice(&first.stdout).expect("readiness JSON");
    assert_eq!(value["readiness_version"], "0.1.0");
    assert_eq!(value["state"], "not_ready");
    assert_eq!(value["targets"][0]["language"], "rust");
    assert_eq!(value["targets"][0]["configured_root"], "missing");
    assert_eq!(value["targets"][0]["detection"]["detected"], false);
    assert!(value["targets"][0]["execution"].is_null());
    assert_eq!(value["targets"][0]["requirements"], serde_json::json!([]));
    assert_eq!(value["issues"][0]["stage"], "detection");
}

#[test]
fn ready_check_uses_human_output_and_returns_zero_without_writes() {
    let tempdir = TempDir::new().expect("tempdir");
    write_policy(tempdir.path(), ".");
    fs::write(
        tempdir.path().join("Cargo.toml"),
        "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .expect("manifest");
    let before = snapshot(tempdir.path());

    let output = check(tempdir.path(), false);

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout).expect("UTF-8 output");
    assert!(stdout.starts_with("Ayni install readiness 0.1.0 — ready\n"));
    assert!(stdout.contains("rust:."));
    assert!(stdout.contains("resolution: runner=cargo"));
    assert_eq!(
        snapshot(tempdir.path()),
        before,
        "check mutated repository paths"
    );
    assert!(!tempdir.path().join(".ayni").exists());
}

#[test]
fn shared_requirements_follow_any_enabled_signal() {
    let tempdir = TempDir::new().expect("tempdir");
    for checks in ["test", "coverage"] {
        let value = check_with_path(tempdir.path(), checks);
        assert_eq!(value["state"], "not_ready");
        assert_eq!(value["targets"].as_array().unwrap().len(), 2);
        assert_eq!(value["targets"][0]["language"], "node");
        assert_eq!(value["targets"][1]["language"], "python");
        assert!(value["targets"][0]["execution"]["runner"].is_string());
        assert!(value["targets"][1]["execution"]["runner"].is_string());
        let node: Vec<_> = value["targets"][0]["requirements"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v["name"].as_str().unwrap())
            .collect();
        let python: Vec<_> = value["targets"][1]["requirements"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v["name"].as_str().unwrap())
            .collect();
        let expected_node = if checks == "test" {
            vec!["node", "vitest"]
        } else {
            vec!["node", "vitest", "@vitest/coverage-v8"]
        };
        let expected_python = if checks == "test" {
            vec!["python", "pytest", "pytest-json-report"]
        } else {
            vec!["python", "pytest", "pytest-cov", "coverage"]
        };
        assert_eq!(node, expected_node);
        assert_eq!(python, expected_python);
        assert!(node.contains(&"vitest"));
        assert!(python.contains(&"pytest"));
    }
}

#[test]
fn spawn_errors_are_requirement_issues_not_absence_or_success() {
    let tempdir = TempDir::new().expect("tempdir");
    let value = check_with_path(tempdir.path(), "test");

    assert_eq!(value["state"], "not_ready");
    let issues = value["issues"].as_array().expect("issues");
    assert!(issues.iter().any(|issue| {
        issue["stage"] == "requirement"
            && issue["requirement"] == "node"
            && issue["message"]
                .as_str()
                .is_some_and(|message| message.contains("kind=spawn"))
    }));
    assert_eq!(value["targets"][0]["requirements"][0]["status"], "missing");
    assert_ne!(value["targets"][0]["requirements"][0]["status"], "current");
}

#[cfg(unix)]
#[test]
fn configured_root_escape_is_rejected_by_install_listing_apply_and_check() {
    use std::os::unix::fs::symlink;

    let tempdir = TempDir::new().expect("tempdir");
    let root = tempdir.path().join("repository");
    let outside = tempdir.path().join("outside");
    fs::create_dir(&root).expect("repository");
    fs::create_dir(&outside).expect("outside");
    fs::write(
        outside.join("Cargo.toml"),
        "[package]\nname = \"outside\"\nversion = \"0.1.0\"\n",
    )
    .expect("outside manifest");
    symlink(&outside, root.join("escape-link")).expect("escape link");
    fs::write(root.join(".gitignore"), ".ayni/\n").expect("gitignore");
    fs::write(
        root.join(".ayni.toml"),
        "[checks]\ntest = true\ncoverage = false\nsize = false\ncomplexity = false\ndeps = false\nmutation = false\n\n[languages]\nenabled = [\"rust\"]\n\n[rust]\nroots = [\"escape-link\"]\n",
    )
    .expect("policy");

    for extra in [&[][..], &["--apply"][..], &["--check"][..]] {
        let output = ayni()
            .args(["install", "--repo-root", "."])
            .args(extra)
            .current_dir(&root)
            .output()
            .expect("launch ayni");
        assert!(!output.status.success(), "{extra:?} unexpectedly succeeded");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("configured root 'escape-link'"), "{stderr}");
        assert!(stderr.contains("repository containment"), "{stderr}");
    }
}

#[test]
fn check_requires_an_existing_valid_policy_and_rejects_invalid_modes() {
    let tempdir = TempDir::new().expect("tempdir");
    let missing = check(tempdir.path(), true);
    assert!(!missing.status.success());
    assert!(missing.stdout.is_empty());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("failed to read"));
    assert!(!tempdir.path().join(".ayni.toml").exists());

    let conflict = ayni()
        .args(["install", "--check", "--apply"])
        .output()
        .expect("launch conflict");
    assert!(!conflict.status.success());
    assert!(conflict.stdout.is_empty());
    assert!(String::from_utf8_lossy(&conflict.stderr).contains("cannot be used with"));

    let output_without_check = ayni()
        .args(["install", "--output", "json"])
        .output()
        .expect("launch invalid output");
    assert!(!output_without_check.status.success());
    assert!(output_without_check.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output_without_check.stderr);
    assert!(stderr.contains("required arguments"));
    assert!(stderr.contains("--check"));
}

#[cfg(unix)]
#[test]
fn timeout_is_not_ready() {
    use std::os::unix::fs::PermissionsExt;

    let tempdir = TempDir::new().expect("tempdir");
    fs::write(
        tempdir.path().join(".ayni.toml"),
        "[checks]\ntest = true\ncoverage = false\nsize = false\ncomplexity = false\ndeps = false\nmutation = false\n\n[languages]\nenabled = [\"rust\"]\n\n[rust]\nroots = [\".\"]\n\n[execution]\ntool_timeout_seconds = 1\n",
    )
    .expect("policy");
    fs::write(
        tempdir.path().join("Cargo.toml"),
        "[package]\nname='fixture'\nversion='0.1.0'\n",
    )
    .expect("manifest");
    let bin = tempdir.path().join("bin");
    fs::create_dir(&bin).expect("bin");
    let cargo = bin.join("cargo");
    fs::write(&cargo, "#!/bin/sh\nsleep 5\n").expect("fake cargo");
    let mut permissions = fs::metadata(&cargo).expect("metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&cargo, permissions).expect("chmod");
    let inherited = std::env::var_os("PATH").unwrap_or_default();
    let path =
        std::env::join_paths(std::iter::once(bin.clone()).chain(std::env::split_paths(&inherited)))
            .expect("PATH");

    let output = ayni()
        .env("PATH", path)
        .args(["install", "--check", "--repo-root", ".", "--output", "json"])
        .current_dir(tempdir.path())
        .output()
        .expect("launch ayni");

    assert!(!output.status.success());
    assert!(output.stderr.is_empty());
    let value: Value = serde_json::from_slice(&output.stdout).expect("readiness JSON");
    assert_eq!(value["state"], "not_ready");
    assert_eq!(value["targets"][0]["requirements"][0]["name"], "cargo");
    assert_eq!(value["targets"][0]["requirements"][0]["status"], "missing");
    assert_eq!(value["issues"][0]["stage"], "requirement");
    assert!(
        value["issues"][0]["message"]
            .as_str()
            .expect("message")
            .contains("kind=timeout")
    );
}
