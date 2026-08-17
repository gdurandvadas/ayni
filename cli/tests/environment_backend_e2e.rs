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
fn contains_materialized_dependency(path: &std::path::Path) -> bool {
    let Ok(entries) = fs::read_dir(path) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.file_name().and_then(|name| name.to_str()) == Some("content")
            && path.ancestors().any(|ancestor| {
                ancestor.file_name().and_then(|name| name.to_str()) == Some("dependencies")
            })
        {
            return true;
        }
        if path.is_dir() && contains_materialized_dependency(&path) {
            return true;
        }
    }
    false
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
        fs::write(target.join("Cargo.lock"), "version = 4\n").unwrap();
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
    let alternate = root.path().join("alternate.toml");
    fs::copy(root.path().join(".ayni.toml"), &alternate).unwrap();
    for arguments in [
        vec!["check"],
        vec!["verify", "test"],
        vec!["impact", "run", "--base", "HEAD"],
    ] {
        let output = command(&root, &arguments)
            .args(["--config"])
            .arg(&alternate)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(3));
        assert!(String::from_utf8_lossy(&output.stderr).contains("lock-bound contract"));
    }
    let fingerprint = lock["fingerprint"].as_str().unwrap();
    let base_digest = lock["provisioning_base"]["digest"].as_str().unwrap();
    let labels = serde_json::json!({
        "dev.ayni.environment.schema": "0.4.0",
        "dev.ayni.environment.lock-fingerprint": fingerprint,
        "dev.ayni.environment.base-digest": base_digest,
        "dev.ayni.environment.ayni-version": env!("CARGO_PKG_VERSION"),
        "dev.ayni.environment.mise-version": "2025.2.4",
        "dev.ayni.environment.platform": if cfg!(target_arch = "aarch64") {
            "linux/arm64"
        } else {
            "linux/amd64"
        },
        "dev.ayni.environment.preparation-digest": "PREPARATION_DIGEST",
    });
    let record = root.path().join("record");
    write_executable(
        &root.path().join("bin/docker"),
        &format!(
            "case \"$1\" in\nversion) echo fake;;\nimage) [ -f '{}.built' ] || exit 1; preparation=$(cat '{}.preparation'); printf '%s\\n' '{}' | sed \"s|PREPARATION_DIGEST|$preparation|g\" ;;\nbuild) printf '%s\\n' \"$@\" > '{}' ; context=''; file=''; shift; while [ $# -gt 0 ]; do if [ \"$1\" = \"--file\" ]; then shift; file=$1; else context=$1; fi; shift; done; cat \"$file\" > '{}.dockerfile'; sed -n 's/.*dev.ayni.environment.preparation-digest=\"\\([^\"]*\\)\".*/\\1/p' \"$file\" > '{}.preparation'; cat \"$context/mise.toml\" > '{}.mise'; find \"$context/repository\" -type f | sed \"s|$context/repository/||\" | sort > '{}.inputs'; touch '{}.built';;\nrun) printf '%s\\n' \"$@\" > '{}.run'; printf '%s\\n' \"$@\" | grep -qx cp && exit 0; exit 7;;\nesac",
            record.display(),
            record.display(),
            labels,
            record.display(),
            record.display(),
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
    assert!(dockerfile.contains(
        "RUN [\"rustup\",\"component\",\"add\",\"--toolchain\",\"1.92.0\",\"llvm-tools-preview\"]"
    ));
    assert!(dockerfile.contains(
        "RUN [\"rustup\",\"component\",\"add\",\"--toolchain\",\"1.93.0\",\"llvm-tools-preview\"]"
    ));
    assert!(dockerfile.contains("FROM ayni-runtime AS ayni-preparation"));
    assert!(dockerfile.contains("\"cargo\",\"fetch\",\"--locked\""));
    assert!(!dockerfile.contains(&root.path().display().to_string()));
    let inputs = fs::read_to_string(format!("{}.inputs", record.display())).unwrap();
    assert_eq!(
        inputs.lines().collect::<Vec<_>>(),
        [
            "one/Cargo.lock",
            "one/Cargo.toml",
            "one/src/lib.rs",
            "one/src/main.rs",
            "two/Cargo.lock",
            "two/Cargo.toml",
            "two/src/lib.rs",
            "two/src/main.rs"
        ]
    );
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
    assert_eq!(run.status.code(), Some(7));
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
    assert!(!recorded.contains("CARGO_HOME="));
    assert!(!recorded.contains("MISE_CACHE_DIR="));
    assert!(recorded.contains("target=/home/ayni/.cache"));
    assert!(recorded.contains(&format!("CARGO_TARGET_DIR={state_home}/targets/")));
    assert!(recorded.contains("MISE_RUST_VERSION=1.93.0"));
    assert!(recorded.contains("--workdir\n/workspace/one"));
    assert!(recorded.contains("RUSTUP_HOME=/home/ayni/.rustup"));
    assert!(recorded.contains("--entrypoint\necho"));
    assert!(recorded.contains("--read-only"));

    let preparation = fs::read_to_string(format!("{}.preparation", record.display())).unwrap();
    let preparation = preparation.trim();
    let preparation = preparation.strip_prefix("sha256:").unwrap_or(preparation);
    let cache_marker = root
        .path()
        .join(".ayni/environment")
        .join(&fingerprint[..16.min(fingerprint.len())])
        .join(&preparation[..16.min(preparation.len())])
        .join("cache.complete");
    fs::remove_file(&cache_marker).unwrap();
    let redirected_marker = root.path().join("redirected-marker");
    fs::write(&redirected_marker, "unchanged").unwrap();
    symlink(&redirected_marker, &cache_marker).unwrap();
    let unsafe_marker = command(&root, &["env", "run", "--repo-root"])
        .arg(root.path())
        .args(["--language", "rust", "--root", "one", "--", "echo", "ok"])
        .output()
        .unwrap();
    assert_eq!(unsafe_marker.status.code(), Some(4));
    assert!(String::from_utf8_lossy(&unsafe_marker.stderr).contains("without following symlinks"));
    assert_eq!(fs::read_to_string(&redirected_marker).unwrap(), "unchanged");
    fs::remove_file(&cache_marker).unwrap();
    fs::write(&cache_marker, format!("sha256:{preparation}")).unwrap();

    write_executable(
        &root.path().join("bin/docker"),
        &format!(
            "case \"$1\" in\nversion) echo fake;;\nimage) preparation=$(cat '{}.preparation'); printf '%s\\n' '{}' | sed \"s|PREPARATION_DIGEST|$preparation|g\" ;;\nrun) printf '%s\\n' \"$@\" > '{}.check'; exit 1;;\nesac",
            record.display(),
            labels,
            record.display()
        ),
    );
    let managed = command(&root, &["check", "--config"])
        .arg(root.path().join(".ayni.toml"))
        .output()
        .unwrap();
    assert_eq!(managed.status.code(), Some(1));
    let managed_args = fs::read_to_string(format!("{}.check", record.display())).unwrap();
    let managed_root = root.path().canonicalize().unwrap();
    assert!(managed_args.contains("AYNI_MANAGED_TARGET_ENVIRONMENTS="));
    assert!(managed_args.contains("rust:one"));
    assert!(managed_args.contains("rust:two"));
    assert!(managed_args.ends_with("--output\nhuman\n"));
    assert!(!managed_args.contains("--entrypoint"));
    assert!(managed_args.contains(&format!(
        "type=bind,source={},target=/workspace,readonly",
        managed_root.display()
    )));
    assert!(managed_args.contains(&format!(
        "type=bind,source={},target=/workspace/.ayni",
        managed_root.join(".ayni").display()
    )));
    assert!(!managed_args.contains(&format!("{}:/workspace:rw", managed_root.display())));
    // The writable nested mount preserves materialized cache/state while the
    // checkout-wide mount remains read-only for managed quality execution.
    assert!(cache_marker.exists());

    let managed_verify = command(&root, &["verify", "coverage", "--config"])
        .arg(root.path().join(".ayni.toml"))
        .args([
            "--language",
            "rust",
            "--root",
            "one",
            "--output",
            "json",
            "--debug",
        ])
        .output()
        .unwrap();
    assert_eq!(managed_verify.status.code(), Some(1));
    let managed_verify_args = fs::read_to_string(format!("{}.check", record.display())).unwrap();
    assert!(managed_verify_args.contains("AYNI_MANAGED_TARGET_ENVIRONMENTS="));
    assert!(managed_verify_args.contains("rust:one"));
    assert!(managed_verify_args.contains("rust:two"));
    assert!(managed_verify_args.ends_with(
        "verify\ncoverage\n--host\n--config\n./.ayni.toml\n--output\njson\n--language\nrust\n--root\none\n--debug\n"
    ));
    assert!(!managed_verify_args.contains("--entrypoint"));
    assert!(managed_verify_args.contains(&format!(
        "type=bind,source={},target=/workspace,readonly",
        managed_root.display()
    )));
    assert!(managed_verify_args.contains(&format!(
        "type=bind,source={},target=/workspace/.ayni",
        managed_root.join(".ayni").display()
    )));
    assert!(cache_marker.exists());

    fs::write(root.path().join("one/rust-toolchain"), "stable\n").unwrap();
    let stale = command(&root, &["env", "doctor", "--repo-root"])
        .arg(root.path())
        .output()
        .unwrap();
    assert_eq!(stale.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&stale.stderr).contains("stale"));
}

#[test]
fn npm_dependencies_are_staged_materialized_offline_and_mounted_for_managed_quality_runs() {
    let root = TempDir::new().unwrap();
    let bin = root.path().join("bin");
    fs::create_dir(&bin).unwrap();
    fs::write(
        root.path().join(".ayni.toml"),
        "[checks]\ntest = false\ncoverage = false\nsize = true\ncomplexity = false\ndeps = false\nmutation = false\n[languages]\nenabled = [\"node\"]\n[node.size]\n\"**/*.js\" = { warn = 100, fail = 200 }\n",
    )
    .unwrap();
    fs::write(
        root.path().join("package.json"),
        r#"{"name":"fixture","version":"1.0.0","packageManager":"npm@10.9.0"}"#,
    )
    .unwrap();
    fs::write(root.path().join(".node-version"), "22.12.0\n").unwrap();
    fs::write(
        root.path().join("package-lock.json"),
        r#"{"name":"fixture","version":"1.0.0","lockfileVersion":3,"requires":true,"packages":{"":{"name":"fixture","version":"1.0.0"}}}"#,
    )
    .unwrap();
    write_executable(
        &bin.join("mise"),
        "while [ \"$1\" = \"--no-config\" ] || [ \"$1\" = \"--no-env\" ] || [ \"$1\" = \"--no-hooks\" ]; do shift; done\n[ \"$1\" = \"version\" ] && echo '2026.8.7 linux-x64' && exit 0\nexit 1",
    );
    write_executable(
        &bin.join("docker"),
        "[ \"$1\" = \"buildx\" ] && printf '{\"digest\":\"sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\"}\\n' && exit 0\nexit 1",
    );
    let locked = command(&root, &["env", "lock", "--repo-root"])
        .arg(root.path())
        .output()
        .unwrap();
    assert!(
        locked.status.success(),
        "{}",
        String::from_utf8_lossy(&locked.stderr)
    );
    let lock: serde_json::Value =
        serde_json::from_slice(&fs::read(root.path().join(".ayni.lock")).unwrap()).unwrap();
    let labels = serde_json::json!({
        "dev.ayni.environment.schema": "0.4.0",
        "dev.ayni.environment.lock-fingerprint": lock["fingerprint"],
        "dev.ayni.environment.base-digest": lock["provisioning_base"]["digest"],
        "dev.ayni.environment.ayni-version": env!("CARGO_PKG_VERSION"),
        "dev.ayni.environment.mise-version": "2025.2.4",
        "dev.ayni.environment.platform": if cfg!(target_arch = "aarch64") { "linux/arm64" } else { "linux/amd64" },
        "dev.ayni.environment.preparation-digest": "PREPARATION_DIGEST",
    });
    let record = root.path().join("node-record");
    write_executable(
        &bin.join("docker"),
        &format!(
            "case \"$1\" in\nversion) echo fake;;\nimage) [ -f '{}.built' ] || exit 1; preparation=$(cat '{}.preparation'); printf '%s\\n' '{}' | sed \"s|PREPARATION_DIGEST|$preparation|g\" ;;\nbuild) context=''; file=''; shift; while [ $# -gt 0 ]; do if [ \"$1\" = \"--file\" ]; then shift; file=$1; else context=$1; fi; shift; done; cat \"$file\" > '{}.dockerfile'; sed -n 's/.*dev.ayni.environment.preparation-digest=\"\\([^\"]*\\)\".*/\\1/p' \"$file\" > '{}.preparation'; find \"$context/repository\" -type f | sed \"s|$context/repository/||\" | sort > '{}.inputs'; touch '{}.built';;\nrun) printf '%s\\n' \"$@\" >> '{}.runs'; if printf '%s\\n' \"$@\" | grep -qx npm && [ ! -f '{}.rebuild-failed' ]; then touch '{}.rebuild-failed'; exit 9; fi; printf '%s\\n' \"$@\" | grep -q -- '--entrypoint' && exit 0; exit 1;;\nesac",
            record.display(),
            record.display(),
            labels,
            record.display(),
            record.display(),
            record.display(),
            record.display(),
            record.display(),
            record.display(),
            record.display()
        ),
    );
    let built = command(&root, &["env", "build", "--repo-root"])
        .arg(root.path())
        .output()
        .unwrap();
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let dockerfile = fs::read_to_string(format!("{}.dockerfile", record.display())).unwrap();
    assert!(dockerfile.contains("\"npm\",\"install\",\"--global\""));
    assert!(dockerfile.contains("\"npm@10.9.0\""));
    assert!(dockerfile.contains("\"npm\",\"ci\",\"--ignore-scripts\""));
    assert!(dockerfile.contains("/opt/ayni/dependencies/"));
    let inputs = fs::read_to_string(format!("{}.inputs", record.display())).unwrap();
    assert_eq!(
        inputs.lines().collect::<Vec<_>>(),
        ["package-lock.json", "package.json"]
    );

    let failed_materialization = command(&root, &["check", "--config"])
        .arg(root.path().join(".ayni.toml"))
        .output()
        .unwrap();
    assert_eq!(failed_materialization.status.code(), Some(4));
    assert!(
        String::from_utf8_lossy(&failed_materialization.stderr)
            .contains("offline dependency materialization command npm failed")
    );
    assert!(!contains_materialized_dependency(
        &root.path().join(".ayni/environment")
    ));
    let managed = command(&root, &["check", "--config"])
        .arg(root.path().join(".ayni.toml"))
        .output()
        .unwrap();
    assert_eq!(managed.status.code(), Some(1));
    let managed_verify = command(&root, &["verify", "size", "--config"])
        .arg(root.path().join(".ayni.toml"))
        .args(["--language", "node", "--output", "json"])
        .output()
        .unwrap();
    assert_eq!(managed_verify.status.code(), Some(1));
    let runs = fs::read_to_string(format!("{}.runs", record.display())).unwrap();
    assert!(runs.contains("npm\nrebuild\n--offline"));
    assert!(runs.contains("target=/workspace/node_modules"));
    assert!(runs.contains("AYNI_MANAGED_TARGET_ENVIRONMENTS="));
    assert!(runs.contains("npm_config_offline"));
    assert!(runs.contains("type=bind"));
    assert!(runs.contains(
        "verify\nsize\n--host\n--config\n./.ayni.toml\n--output\njson\n--language\nnode"
    ));
}

#[test]
fn pnpm_workspace_materializes_all_node_modules_trees_in_one_offline_run() {
    let root = TempDir::new().unwrap();
    let bin = root.path().join("bin");
    fs::create_dir(&bin).unwrap();
    fs::create_dir_all(root.path().join("packages/app")).unwrap();
    fs::create_dir_all(root.path().join("packages/ui")).unwrap();
    fs::write(
        root.path().join(".ayni.toml"),
        "[checks]\ntest = false\ncoverage = false\nsize = true\ncomplexity = false\ndeps = false\nmutation = false\n[languages]\nenabled = [\"node\"]\n[node.size]\n\"**/*.js\" = { warn = 100, fail = 200 }\n",
    )
    .unwrap();
    fs::write(root.path().join(".node-version"), "24.14.0\n").unwrap();
    fs::write(
        root.path().join("package.json"),
        r#"{"name":"fixture","private":true,"packageManager":"pnpm@11.15.1","workspaces":["packages/*"]}"#,
    )
    .unwrap();
    fs::write(
        root.path().join("packages/app/package.json"),
        r#"{"name":"app","version":"1.0.0"}"#,
    )
    .unwrap();
    fs::write(
        root.path().join("packages/ui/package.json"),
        r#"{"name":"ui","version":"1.0.0"}"#,
    )
    .unwrap();
    fs::write(
        root.path().join("pnpm-lock.yaml"),
        "lockfileVersion: '9.0'\nimporters:\n  .: {}\n  packages/app: {}\n  packages/ui: {}\n",
    )
    .unwrap();
    write_executable(
        &bin.join("mise"),
        "while [ \"$1\" = \"--no-config\" ] || [ \"$1\" = \"--no-env\" ] || [ \"$1\" = \"--no-hooks\" ]; do shift; done\n[ \"$1\" = \"version\" ] && echo '2026.8.7 linux-x64' && exit 0\nexit 1",
    );
    write_executable(
        &bin.join("docker"),
        "[ \"$1\" = \"buildx\" ] && printf '{\"digest\":\"sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\"}\\n' && exit 0\nexit 1",
    );
    let locked = command(&root, &["env", "lock", "--repo-root"])
        .arg(root.path())
        .output()
        .unwrap();
    assert!(
        locked.status.success(),
        "{}",
        String::from_utf8_lossy(&locked.stderr)
    );
    let lock: serde_json::Value =
        serde_json::from_slice(&fs::read(root.path().join(".ayni.lock")).unwrap()).unwrap();
    let labels = serde_json::json!({
        "dev.ayni.environment.schema": "0.4.0",
        "dev.ayni.environment.lock-fingerprint": lock["fingerprint"],
        "dev.ayni.environment.base-digest": lock["provisioning_base"]["digest"],
        "dev.ayni.environment.ayni-version": env!("CARGO_PKG_VERSION"),
        "dev.ayni.environment.mise-version": "2025.2.4",
        "dev.ayni.environment.platform": if cfg!(target_arch = "aarch64") { "linux/arm64" } else { "linux/amd64" },
        "dev.ayni.environment.preparation-digest": "PREPARATION_DIGEST",
    });
    let record = root.path().join("pnpm-record");
    write_executable(
        &bin.join("docker"),
        &format!(
            "case \"$1\" in\nversion) echo fake;;\nimage) [ -f '{}.built' ] || exit 1; preparation=$(cat '{}.preparation'); printf '%s\\n' '{}' | sed \"s|PREPARATION_DIGEST|$preparation|g\" ;;\nbuild) context=''; file=''; shift; while [ $# -gt 0 ]; do if [ \"$1\" = \"--file\" ]; then shift; file=$1; else context=$1; fi; shift; done; cat \"$file\" > '{}.dockerfile'; sed -n 's/.*dev.ayni.environment.preparation-digest=\"\\([^\"]*\\)\".*/\\1/p' \"$file\" > '{}.preparation'; touch '{}.built';;\nrun) {{ echo BEGIN; printf '%s\\n' \"$@\"; echo END; }} >> '{}.runs'; printf '%s\\n' \"$@\" | grep -q -- '--entrypoint' && exit 0; exit 1;;\nesac",
            record.display(),
            record.display(),
            labels,
            record.display(),
            record.display(),
            record.display(),
            record.display(),
        ),
    );
    let built = command(&root, &["env", "build", "--repo-root"])
        .arg(root.path())
        .output()
        .unwrap();
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let dockerfile = fs::read_to_string(format!("{}.dockerfile", record.display())).unwrap();
    assert!(dockerfile.contains("\"mise\",\"exec\",\"node@24.14.0\""));
    assert!(dockerfile.contains("\"pnpm@11.15.1\""));
    assert!(!dockerfile.contains("\"mise\",\"install\",\"--yes\",\"pnpm@"));

    let managed = command(&root, &["check", "--config"])
        .arg(root.path().join(".ayni.toml"))
        .output()
        .unwrap();
    assert_eq!(managed.status.code(), Some(1));
    let runs = fs::read_to_string(format!("{}.runs", record.display())).unwrap();
    let rebuilds = runs
        .split("BEGIN\n")
        .filter(|run| run.lines().any(|line| line == "pnpm"))
        .collect::<Vec<_>>();
    assert_eq!(rebuilds.len(), 1, "{runs}");
    let rebuild = rebuilds[0];
    assert!(rebuild.contains("target=/workspace/node_modules"));
    assert!(rebuild.contains("target=/workspace/packages/app/node_modules"));
    assert!(rebuild.contains("target=/workspace/packages/ui/node_modules"));
    assert!(rebuild.contains("XDG_STATE_HOME=/tmp/ayni/xdg-state"));
    assert!(rebuild.contains("XDG_DATA_HOME=/tmp/ayni/xdg-data"));
    assert!(!rebuild.contains("target=/workspace,readonly"));
    assert!(runs.contains("AYNI_MANAGED_TARGET_ENVIRONMENTS="));
    assert!(runs.contains("PNPM_CONFIG_VERIFY_DEPS_BEFORE_RUN"));
}

#[test]
fn five_language_build_composes_preparation_without_staging_source() {
    let root = TempDir::new().expect("tempdir");
    let bin = root.path().join("bin");
    fs::create_dir(&bin).unwrap();
    fs::write(
        root.path().join(".ayni.toml"),
        "[checks]\ntest=true\ncoverage=false\nsize=false\ncomplexity=false\ndeps=false\nmutation=false\n[languages]\nenabled=[\"rust\",\"node\",\"go\",\"python\",\"kotlin\"]\n[rust]\nroots=[\"rust\"]\n[node]\nroots=[\"node\"]\n[go]\nroots=[\"go\"]\n[python]\nroots=[\"python\"]\n[kotlin]\nroots=[\"kotlin\"]\n",
    )
    .unwrap();
    fs::create_dir(root.path().join("rust")).unwrap();
    fs::write(
        root.path().join("rust/Cargo.toml"),
        "[package]\nname='rust-fixture'\nversion='0.1.0'\nrust-version='1.97.1'\n",
    )
    .unwrap();
    fs::write(
        root.path().join("rust/rust-toolchain.toml"),
        "[toolchain]\nchannel='1.97.1'\n",
    )
    .unwrap();
    fs::write(root.path().join("rust/Cargo.lock"), "version = 4\n").unwrap();
    fs::write(root.path().join("rust/lib.rs"), "pub fn source() {}\n").unwrap();

    fs::create_dir(root.path().join("node")).unwrap();
    fs::write(
        root.path().join("node/package.json"),
        r#"{"name":"node-fixture","private":true,"packageManager":"npm@10.9.3","engines":{"node":"22.14.0"},"devDependencies":{"vitest":"3.2.4"}}"#,
    )
    .unwrap();
    fs::write(root.path().join("node/.node-version"), "22.14.0\n").unwrap();
    fs::write(
        root.path().join("node/package-lock.json"),
        r#"{"name":"node-fixture","lockfileVersion":3,"requires":true,"packages":{"":{"name":"node-fixture","devDependencies":{"vitest":"3.2.4"},"engines":{"node":"22.14.0"}},"node_modules/vitest":{"version":"3.2.4"}}}"#,
    )
    .unwrap();
    fs::write(
        root.path().join("node/app.js"),
        "export const source = true;\n",
    )
    .unwrap();

    fs::create_dir(root.path().join("go")).unwrap();
    fs::write(
        root.path().join("go/go.mod"),
        "module example.com/go\n\ngo 1.23\ntoolchain go1.23.4\n",
    )
    .unwrap();
    fs::write(root.path().join("go/main.go"), "package main\n").unwrap();

    fs::create_dir(root.path().join("python")).unwrap();
    fs::write(
        root.path().join("python/pyproject.toml"),
        "[project]\nname='python-fixture'\nrequires-python='>=3.12'\n[dependency-groups]\ndev=['pytest==8.3.5','pytest-json-report==1.5.0']\n[tool.uv]\nrequired-version='0.6.0'\n",
    )
    .unwrap();
    fs::write(root.path().join("python/.python-version"), "3.12.4\n").unwrap();
    fs::write(
        root.path().join("python/uv.lock"),
        "version=1\n[[package]]\nname='pytest'\nversion='8.3.5'\n[[package]]\nname='pytest-json-report'\nversion='1.5.0'\n",
    )
    .unwrap();
    fs::write(root.path().join("python/app.py"), "print('source')\n").unwrap();

    fs::create_dir_all(root.path().join("kotlin/gradle/wrapper")).unwrap();
    fs::write(
        root.path().join("kotlin/settings.gradle.kts"),
        "rootProject.name=\"fixture\"\n",
    )
    .unwrap();
    fs::write(
        root.path().join("kotlin/build.gradle.kts"),
        "plugins { kotlin(\"jvm\") version \"2.1.0\" }\n",
    )
    .unwrap();
    fs::write(root.path().join("kotlin/.java-version"), "21.0.6\n").unwrap();
    fs::write(root.path().join("kotlin/gradlew"), "#!/bin/sh\nexit 0\n").unwrap();
    fs::write(
        root.path().join("kotlin/gradle/wrapper/gradle-wrapper.jar"),
        "jar",
    )
    .unwrap();
    fs::write(
        root.path().join("kotlin/gradle/wrapper/gradle-wrapper.properties"),
        format!("distributionUrl=https\\://services.gradle.org/distributions/gradle-8.10.2-bin.zip\ndistributionSha256Sum={}\n", "a".repeat(64)),
    )
    .unwrap();
    fs::write(
        root.path().join("kotlin/gradle.lockfile"),
        "example:dependency:1.0=runtimeClasspath\n",
    )
    .unwrap();
    fs::create_dir_all(root.path().join("kotlin/src/main/kotlin")).unwrap();
    fs::write(
        root.path().join("kotlin/src/main/kotlin/App.kt"),
        "class App\n",
    )
    .unwrap();

    write_executable(
        &bin.join("mise"),
        "while [ \"$1\" = \"--no-config\" ] || [ \"$1\" = \"--no-env\" ] || [ \"$1\" = \"--no-hooks\" ]; do shift; done\n[ \"$1\" = \"version\" ] && echo '2026.8.7 linux-x64' && exit 0\nexit 1",
    );
    let record = root.path().join("polyglot");
    write_executable(
        &bin.join("docker"),
        &format!(
            "case \"$1\" in\nbuildx) printf '{{\"digest\":\"sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\"}}\\n';;\nimage) exit 1;;\nbuild) shift; file=''; context=''; while [ $# -gt 0 ]; do if [ \"$1\" = \"--file\" ]; then shift; file=$1; else context=$1; fi; shift; done; cat \"$file\" > '{}.dockerfile'; cat \"$context/mise.toml\" > '{}.mise'; find \"$context/repository\" -type f | sed \"s|$context/repository/||\" | sort > '{}.inputs';;\nesac\nexit 0",
            record.display(),
            record.display(),
            record.display(),
        ),
    );
    let locked = command(&root, &["env", "lock", "--repo-root"])
        .arg(root.path())
        .output()
        .unwrap();
    assert!(
        locked.status.success(),
        "{}",
        String::from_utf8_lossy(&locked.stderr)
    );
    let lock: serde_json::Value =
        serde_json::from_slice(&fs::read(root.path().join(".ayni.lock")).unwrap()).unwrap();
    let labels = serde_json::json!({
        "dev.ayni.environment.schema": "0.4.0",
        "dev.ayni.environment.lock-fingerprint": lock["fingerprint"],
        "dev.ayni.environment.base-digest": lock["provisioning_base"]["digest"],
        "dev.ayni.environment.ayni-version": env!("CARGO_PKG_VERSION"),
        "dev.ayni.environment.mise-version": "2025.2.4",
        "dev.ayni.environment.platform": if cfg!(target_arch = "aarch64") {
            "linux/arm64"
        } else {
            "linux/amd64"
        },
        "dev.ayni.environment.preparation-digest": "PREPARATION_DIGEST",
    });
    write_executable(
        &bin.join("docker"),
        &format!(
            "case \"$1\" in\nimage) [ -f '{}.built' ] || exit 1; preparation=$(cat '{}.preparation'); printf '%s\\n' '{}' | sed \"s|PREPARATION_DIGEST|$preparation|g\";;\nbuild) shift; file=''; context=''; while [ $# -gt 0 ]; do if [ \"$1\" = \"--file\" ]; then shift; file=$1; else context=$1; fi; shift; done; cat \"$file\" > '{}.dockerfile'; sed -n 's/.*dev.ayni.environment.preparation-digest=\"\\([^\"]*\\)\".*/\\1/p' \"$file\" > '{}.preparation'; cat \"$context/mise.toml\" > '{}.mise'; find \"$context/repository\" -type f | sed \"s|$context/repository/||\" | sort > '{}.inputs'; touch '{}.built';;\nrun) printf '%s\\n' \"$@\" > '{}.run';;\nesac\nexit 0",
            record.display(),
            record.display(),
            labels,
            record.display(),
            record.display(),
            record.display(),
            record.display(),
            record.display(),
            record.display(),
        ),
    );
    let built = command(&root, &["env", "build", "--repo-root"])
        .arg(root.path())
        .output()
        .unwrap();
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let dockerfile = fs::read_to_string(format!("{}.dockerfile", record.display())).unwrap();
    assert!(dockerfile.contains("\"cargo\",\"fetch\",\"--locked\""));
    assert!(dockerfile.contains("\"npm\",\"install\",\"--global\""));
    assert!(dockerfile.contains("\"npm@10.9.3\""));
    assert!(dockerfile.contains("\"npm\",\"ci\",\"--ignore-scripts\""));
    assert!(dockerfile.contains("\"go\",\"mod\",\"download\",\"all\""));
    assert!(dockerfile.contains("\"uv\",\"sync\",\"--frozen\",\"--no-install-project\""));
    assert!(dockerfile.contains("\"sh\",\"gradlew\",\"--no-daemon\""));
    assert!(dockerfile.contains(".ayni-gradle-resolve.init.gradle"));
    let mise = fs::read_to_string(format!("{}.mise", record.display())).unwrap();
    for tool in ["rust", "node", "go", "python", "uv", "java", "gradle"] {
        assert!(
            mise.contains(&format!("\"{tool}\"")),
            "missing {tool}: {mise}"
        );
    }
    let inputs = fs::read_to_string(format!("{}.inputs", record.display())).unwrap();
    assert!(inputs.contains("rust/Cargo.toml"));
    assert!(inputs.contains("rust/Cargo.lock"));
    assert!(inputs.contains("node/package.json"));
    assert!(inputs.contains("node/package-lock.json"));
    assert!(inputs.contains("go/go.mod"));
    assert!(inputs.contains("python/pyproject.toml"));
    assert!(inputs.contains("python/uv.lock"));
    assert!(inputs.contains("kotlin/gradle.lockfile"));
    assert!(inputs.contains("kotlin/.ayni-gradle-resolve.init.gradle"));
    assert!(!inputs.contains("rust/lib.rs"));
    assert!(!inputs.contains("node/app.js"));
    assert!(!inputs.contains("go/main.go"));
    assert!(!inputs.contains("python/app.py"));
    assert!(!inputs.contains("kotlin/src/main/kotlin/App.kt"));

    let managed = command(&root, &["check", "--config"])
        .arg(root.path().join(".ayni.toml"))
        .output()
        .unwrap();
    assert!(
        managed.status.success(),
        "{}",
        String::from_utf8_lossy(&managed.stderr)
    );
    let run = fs::read_to_string(format!("{}.run", record.display())).unwrap();
    assert!(run.contains("AYNI_MANAGED_TARGET_ENVIRONMENTS="));
    assert!(run.contains("AYNI_GRADLE_OUTPUT_ROOT"));
    assert!(run.contains("/workspace/.ayni/quality/kotlin/6b6f746c696e"));
    assert!(run.contains("target=/workspace,readonly"));
    assert!(run.contains("target=/workspace/.ayni"));
}
