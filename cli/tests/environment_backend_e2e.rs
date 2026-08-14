#![cfg(unix)]
use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::process::Command;
use tempfile::TempDir;

fn write_executable(path: &std::path::Path, body: &str) {
    fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
}
fn fixture() -> TempDir {
    let root = TempDir::new().unwrap();
    let bin = root.path().join("bin");
    fs::create_dir(&bin).unwrap();
    fs::write(root.path().join(".ayni.toml"), "[checks]\ntest = true\ncoverage = true\nsize = false\ncomplexity = false\ndeps = false\nmutation = false\n[languages]\nenabled = [\"rust\"]\n[rust]\nroots = [\"one\", \"two\"]\n").unwrap();
    for (path, version) in [("one", "1.93.0"), ("two", "1.92.0")] {
        let target = root.path().join(path);
        fs::create_dir(&target).unwrap();
        fs::write(
            target.join("Cargo.toml"),
            format!(
                "[package]\nname = \"fixture-{path}\"\nversion = \"0.1.0\"\nrust-version = \"{version}\"\n"
            ),
        )
        .unwrap();
        fs::write(
            target.join("rust-toolchain.toml"),
            format!("[toolchain]\nchannel = \"{version}\"\n"),
        )
        .unwrap();
    }
    write_executable(
        &bin.join("mise"),
        "while [ \"$1\" = \"--no-config\" ] || [ \"$1\" = \"--no-env\" ] || [ \"$1\" = \"--no-hooks\" ]; do shift; done\n[ \"$1\" = \"version\" ] && echo '2026.8.7 linux-x64' && exit 0\nexit 1",
    );
    write_executable(
        &bin.join("docker"),
        "[ \"$1\" = \"buildx\" ] && printf '{\"digest\":\"sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\"}\\n' && exit 0\nexit 1",
    );
    root
}
fn command(root: &TempDir, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_ayni"));
    command.args(args);
    let path = std::env::join_paths(std::iter::once(root.path().join("bin")).chain(
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()),
    ))
    .unwrap();
    command.env("PATH", path);
    command
}
#[test]
fn build_and_run_use_a_fake_docker_without_baking_the_checkout() {
    let root = fixture();
    let output = command(&root, &["env", "lock", "--repo-root"])
        .arg(root.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let lock: serde_json::Value =
        serde_json::from_slice(&fs::read(root.path().join(".ayni.lock")).unwrap()).unwrap();
    let fingerprint = lock["fingerprint"].as_str().unwrap();
    let base_digest = lock["provisioning_base"]["digest"].as_str().unwrap();
    let labels = serde_json::json!({
        "dev.ayni.environment.schema": "0.2.0",
        "dev.ayni.environment.lock-fingerprint": fingerprint,
        "dev.ayni.environment.base-digest": base_digest,
        "dev.ayni.environment.ayni-version": env!("CARGO_PKG_VERSION"),
        "dev.ayni.environment.mise-version": "2025.2.4",
        "dev.ayni.environment.platform": if cfg!(target_arch = "aarch64") {
            "linux/arm64"
        } else {
            "linux/amd64"
        },
    });
    let record = root.path().join("record");
    write_executable(
        &root.path().join("bin/docker"),
        &format!(
            "case \"$1\" in\nversion) echo fake;;\nimage) [ -f '{}.built' ] || exit 1; printf '%s\\n' '{}' ;;\nbuild) printf '%s\\n' \"$@\" > '{}' ; context=''; file=''; shift; while [ $# -gt 0 ]; do if [ \"$1\" = \"--file\" ]; then shift; file=$1; else context=$1; fi; shift; done; cat \"$file\" > '{}.dockerfile'; cat \"$context/mise.toml\" > '{}.mise'; touch '{}.built';;\nrun) printf '%s\\n' \"$@\" > '{}.run'; exit 7;;\nesac",
            record.display(),
            labels,
            record.display(),
            record.display(),
            record.display(),
            record.display(),
            record.display()
        ),
    );
    let build = command(&root, &["env", "build", "--repo-root"])
        .arg(root.path())
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let doctor = command(&root, &["env", "doctor", "--repo-root"])
        .arg(root.path())
        .output()
        .unwrap();
    assert!(
        doctor.status.success(),
        "{}",
        String::from_utf8_lossy(&doctor.stderr)
    );
    let dockerfile = fs::read_to_string(format!("{}.dockerfile", record.display())).unwrap();
    assert!(dockerfile.contains("MISE_AUTO_INSTALL=0"));
    assert!(dockerfile.contains("MISE_RUSTUP_COMPONENTS=\"llvm-tools-preview\""));
    assert!(!dockerfile.contains(&root.path().display().to_string()));
    let mise = fs::read_to_string(format!("{}.mise", record.display())).unwrap();
    assert!(mise.starts_with("[tools]\n"));
    assert!(mise.contains("\"rust\" = [\"1.92.0\", \"1.93.0\"]"));
    assert!(mise.contains("\"cargo:cargo-llvm-cov\" = \"0.8.5\""));
    let ambiguous = command(&root, &["env", "run", "--repo-root"])
        .arg(root.path())
        .args(["--", "echo", "ok"])
        .output()
        .unwrap();
    assert_eq!(ambiguous.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&ambiguous.stderr).contains("multiple targets"));

    let escaped_state = TempDir::new().unwrap();
    symlink(escaped_state.path(), root.path().join(".ayni")).unwrap();
    let escaped = command(&root, &["env", "run", "--repo-root"])
        .arg(root.path())
        .args(["--language", "rust", "--root", "one", "--", "echo", "ok"])
        .output()
        .unwrap();
    assert_eq!(escaped.status.code(), Some(4));
    assert!(String::from_utf8_lossy(&escaped.stderr).contains("must not contain symlinks"));
    assert!(!escaped_state.path().join("environment").exists());
    fs::remove_file(root.path().join(".ayni")).unwrap();

    let run = command(&root, &["env", "run", "--repo-root"])
        .arg(root.path())
        .args(["--language", "rust", "--root", "one", "--", "echo", "ok"])
        .output()
        .unwrap();
    assert_eq!(run.status.code(), Some(4));
    assert!(String::from_utf8_lossy(&run.stderr).contains("exited with code 7"));
    let recorded = fs::read_to_string(format!("{}.run", record.display())).unwrap();
    assert!(recorded.contains("--network\nnone"));
    assert!(recorded.contains("/workspace:rw"));
    assert!(recorded.contains("MISE_AUTO_INSTALL=0"));
    let fingerprint = fingerprint.strip_prefix("sha256:").unwrap_or(fingerprint);
    let state_home = format!(
        "/workspace/.ayni/environment/{}/home",
        &fingerprint[..16.min(fingerprint.len())]
    );
    assert!(recorded.contains(&format!("CARGO_HOME={state_home}/.cache/cargo")));
    assert!(recorded.contains(&format!("MISE_CACHE_DIR={state_home}/.cache/mise")));
    assert!(recorded.contains("MISE_RUST_VERSION=1.93.0"));
    assert!(recorded.contains("--workdir\n/workspace/one"));
    assert!(recorded.contains("RUSTUP_HOME=/home/ayni/.rustup"));
    assert!(recorded.contains("--entrypoint\necho"));
    assert!(recorded.contains("--read-only"));

    fs::write(root.path().join("one/rust-toolchain"), "stable\n").unwrap();
    let stale = command(&root, &["env", "doctor", "--repo-root"])
        .arg(root.path())
        .output()
        .unwrap();
    assert_eq!(stale.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&stale.stderr).contains("stale"));
}
