#![cfg(unix)]

use serde_json::Value;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;

fn ayni() -> Command {
    Command::new(env!("CARGO_BIN_EXE_ayni"))
}

fn executable(path: &Path, body: &str) {
    fs::write(path, body).expect("write executable");
    let mut permissions = fs::metadata(path).expect("metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).expect("chmod");
}

fn fixture_path(bin: &Path) -> std::ffi::OsString {
    let inherited = std::env::var_os("PATH").unwrap_or_default();
    std::env::join_paths(
        std::iter::once(bin.to_path_buf()).chain(std::env::split_paths(&inherited)),
    )
    .expect("PATH")
}

fn run(root: &Path, bin: &Path, args: &[&str]) -> Output {
    ayni()
        .env("PATH", fixture_path(bin))
        .args(["install", "--repo-root", "."])
        .args(args)
        .current_dir(root)
        .output()
        .expect("launch ayni")
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn policy(language: &str, configured_root: &str) -> String {
    format!(
        "[checks]\ntest = true\ncoverage = false\nsize = false\ncomplexity = false\ndeps = false\nmutation = false\n\n[languages]\nenabled = [\"{language}\"]\n\n[{language}]\nroots = [\"{configured_root}\"]\n\n[execution]\ntool_timeout_seconds = 2\n"
    )
}

fn prepare_root(language: &str, workspace: bool) -> (TempDir, PathBuf, PathBuf, PathBuf) {
    let temp = TempDir::new().expect("tempdir");
    let root = temp.path().to_path_buf();
    let target = if workspace {
        let target = root.join("packages/app");
        fs::create_dir_all(&target).expect("target");
        target
    } else {
        root.clone()
    };
    let configured = if workspace { "packages/app" } else { "." };
    fs::write(root.join(".ayni.toml"), policy(language, configured)).expect("policy");
    fs::write(root.join(".gitignore"), ".ayni/\n").expect("gitignore");
    let bin = root.join("bin");
    fs::create_dir(&bin).expect("bin");
    let log = root.join("manager.log");
    (temp, target, bin, log)
}

fn setup_node(workspace: bool) -> (TempDir, PathBuf, PathBuf, PathBuf) {
    let (temp, target, bin, log) = prepare_root("node", workspace);
    let root = temp.path();
    let manifest = r#"{"devDependencies":{"vitest":"^3.2.4"}}"#;
    fs::write(target.join("package.json"), manifest).expect("target manifest");
    if workspace {
        fs::write(
            root.join("package.json"),
            r#"{"workspaces":["packages/*"],"packageManager":"npm@10","devDependencies":{"vitest":"^3.2.4"}}"#,
        )
        .expect("workspace manifest");
    } else {
        fs::write(target.join("package-lock.json"), "").expect("lock");
    }
    executable(
        &bin.join("node"),
        &format!(
            "#!/bin/sh\nprintf '%s|node %s\\n' \"$PWD\" \"$*\" >> '{}'\nprintf 'v22.0.0\\n'\n",
            log.display()
        ),
    );
    executable(
        &bin.join("npm"),
        &format!(
            "#!/bin/sh\nprintf '%s|npm %s\\n' \"$PWD\" \"$*\" >> '{}'\nif [ \"$1\" = install ] && [ \"$2\" = --save-dev ]; then /bin/mkdir -p node_modules/vitest; printf '{{}}\\n' > node_modules/vitest/package.json; fi\nprintf 'progress:%s\\n' \"$*\"\n",
            log.display()
        ),
    );
    (temp, target, bin, log)
}

fn setup_python(workspace: bool) -> (TempDir, PathBuf, PathBuf, PathBuf) {
    let (temp, target, bin, log) = prepare_root("python", workspace);
    let root = temp.path();
    fs::write(target.join("pyproject.toml"), "[project]\nname='app'\n").expect("manifest");
    if workspace {
        fs::write(
            root.join("pyproject.toml"),
            "[tool.uv.workspace]\nmembers=['packages/*']\n",
        )
        .expect("workspace manifest");
        fs::write(root.join("uv.lock"), "").expect("uv lock");
    }
    let python = format!(
        "#!/bin/sh\nprintf '%s|python %s\\n' \"$PWD\" \"$*\" >> '{}'\ncase \"$*\" in\n  *'pip install pytest-json-report'*) : > .ayni-fake-pytest-json;;\n  *'pip install pytest'*) : > .ayni-fake-pytest;;\n  *pytest_jsonreport*) [ -f .ayni-fake-pytest-json ] && exit 0 || exit 1;;\n  *pytest*) [ -f .ayni-fake-pytest ] && exit 0 || exit 1;;\nesac\nprintf 'progress:%s\\n' \"$*\"\nexit 0\n",
        log.display()
    );
    executable(&bin.join("python"), &python);
    executable(&bin.join("python3"), &python);
    executable(
        &bin.join("uv"),
        &format!(
            "#!/bin/sh\nprintf '%s|uv %s\\n' \"$PWD\" \"$*\" >> '{}'\ncase \"$*\" in\n  'add --dev pytest-json-report') : > .ayni-fake-pytest-json;;\n  'add --dev pytest') : > .ayni-fake-pytest;;\n  *pytest_jsonreport*) [ -f .ayni-fake-pytest-json ] && exit 0 || exit 1;;\n  *pytest*) [ -f .ayni-fake-pytest ] && exit 0 || exit 1;;\nesac\nprintf 'progress:%s\\n' \"$*\"\nexit 0\n",
            log.display()
        ),
    );
    (temp, target, bin, log)
}

fn exercise_matrix(setup: fn(bool) -> (TempDir, PathBuf, PathBuf, PathBuf), language: &str) {
    for workspace in [false, true] {
        let (temp, _target, bin, log) = setup(workspace);
        let root = temp.path();

        let listing = run(root, &bin, &[]);
        assert_success(&listing);
        let listing_log = fs::read_to_string(&log).unwrap_or_default();
        assert!(!listing_log.contains("install --save-dev"));
        assert!(!listing_log.contains("add --dev"));
        let before_check = listing_log.clone();

        let check = run(root, &bin, &["--check", "--output", "json"]);
        assert!(!check.status.success(), "requirements begin missing");
        assert!(check.stderr.is_empty());
        let readiness: Value = serde_json::from_slice(&check.stdout).expect("readiness");
        assert_eq!(readiness["readiness_version"], "0.1.0");
        assert_eq!(readiness["state"], "not_ready");
        let check_log = fs::read_to_string(&log).unwrap_or_default();
        let check_delta = &check_log[before_check.len()..];
        assert!(!check_delta.contains("install --save-dev"));
        assert!(!check_delta.contains("add --dev"));

        let apply = run(root, &bin, &["--apply"]);
        assert_success(&apply);
        let stdout = String::from_utf8_lossy(&apply.stdout);
        assert!(
            stdout.contains("progress:"),
            "live progress missing: {stdout}"
        );
        let apply_log = fs::read_to_string(&log).expect("apply log");
        let canonical = root.canonicalize().expect("canonical root");
        assert!(
            apply_log
                .lines()
                .all(|line| line.starts_with(&canonical.to_string_lossy().into_owned())),
            "install/status must use install_cwd: {apply_log}"
        );
        if language == "node" {
            assert_eq!(
                apply_log.matches("|npm install\n").count(),
                1,
                "prepare once"
            );
            assert!(apply_log.contains("npm install --save-dev vitest@3.2.4"));
        } else if workspace {
            assert!(apply_log.contains("uv add --dev pytest"));
            assert!(apply_log.contains("uv add --dev pytest-json-report"));
        } else {
            assert!(apply_log.contains("python -m pip install pytest"));
            assert!(apply_log.contains("python -m pip install pytest-json-report"));
        }

        let ready = run(root, &bin, &["--check", "--output", "json"]);
        assert_success(&ready);
        let readiness: Value = serde_json::from_slice(&ready.stdout).expect("ready JSON");
        assert_eq!(readiness["state"], "ready");
    }
}

#[test]
fn node_list_apply_check_workspace_matrix() {
    exercise_matrix(setup_node, "node");
}

#[test]
fn python_list_apply_check_workspace_matrix() {
    exercise_matrix(setup_python, "python");
}
