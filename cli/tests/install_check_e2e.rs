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
