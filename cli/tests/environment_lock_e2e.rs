#![cfg(unix)]

use std::fs;
use std::process::{Command, Output};
use tempfile::TempDir;

fn lock(root: &TempDir) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_ayni"));
    command
        .args(["env", "lock", "--repo-root"])
        .arg(root.path());
    let bin = root.path().join("bin");
    if bin.is_dir() {
        let inherited = std::env::var_os("PATH").unwrap_or_default();
        let path =
            std::env::join_paths(std::iter::once(bin).chain(std::env::split_paths(&inherited)))
                .unwrap();
        command.env("PATH", path);
    }
    command.output().expect("launch ayni")
}

fn fixture() -> TempDir {
    let root = TempDir::new().expect("tempdir");
    fs::write(root.path().join(".ayni.toml"), "[checks]\ntest = true\ncoverage = false\nsize = false\ncomplexity = false\ndeps = false\nmutation = false\n[languages]\nenabled = [\"rust\"]\n[environment.tools]\nprotoc = \"35.1\"\n[environment.debian]\npackages = [\"libssl-dev\"]\n[environment.docker]\naccess = \"socket\"\nnetwork = \"bridge\"\n").unwrap();
    fs::write(
        root.path().join("Cargo.toml"),
        "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nrust-version = \"1.93.0\"\n",
    )
    .unwrap();
    fs::write(
        root.path().join("rust-toolchain.toml"),
        "[toolchain]\nchannel = \"1.93.0\"\n",
    )
    .unwrap();
    fake_mise(
        &root,
        r#"if [ "$1" = "version" ]; then echo "2026.8.7 linux-x64"; exit 0; fi
exit 1"#,
    );
    root
}

#[test]
fn writes_a_valid_deterministic_lock_without_generated_state() {
    let root = fixture();
    let first = lock(&root);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let path = root.path().join(".ayni.lock");
    let one = fs::read(&path).expect("lock");
    let parsed: ayni_core::EnvironmentLock = serde_json::from_slice(&one).expect("valid lock");
    assert_eq!(parsed.targets().len(), 1);
    assert!(parsed.targets()[0].runtimes[0].source.digest.is_some());
    assert_eq!(parsed.tools()[0].tool, "protoc");
    assert_eq!(
        parsed
            .debian_packages()
            .iter()
            .map(|package| package.package.as_str())
            .collect::<Vec<_>>(),
        ["docker.io", "libssl-dev"]
    );
    assert_eq!(
        parsed.capabilities().docker,
        ayni_core::DockerAccess::Socket
    );
    assert_eq!(
        parsed.capabilities().network,
        ayni_core::NetworkAccess::Bridge
    );
    assert!(!root.path().join(".ayni").exists());
    let second = lock(&root);
    assert!(second.status.success());
    assert_eq!(one, fs::read(path).unwrap());
    assert!(String::from_utf8_lossy(&second.stdout).contains("current"));
}

#[test]
fn failure_preserves_an_existing_lock() {
    let root = fixture();
    assert!(lock(&root).status.success());
    let path = root.path().join(".ayni.lock");
    let existing = fs::read(&path).unwrap();
    fs::write(
        root.path().join("rust-toolchain"),
        "definitely-not-a-version\n",
    )
    .unwrap();
    let output = lock(&root);
    assert_eq!(output.status.code(), Some(3));
    assert_eq!(existing, fs::read(path).unwrap());
}

#[test]
fn older_schema_lock_is_replaced_without_manual_deletion() {
    let root = fixture();
    let path = root.path().join(".ayni.lock");
    fs::write(&path, "{\"schema_version\":\"0.3.0\"}\n").unwrap();
    let output = lock(&root);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let parsed: ayni_core::EnvironmentLock =
        serde_json::from_slice(&fs::read(path).unwrap()).expect("current lock");
    assert_eq!(
        ayni_core::ENVIRONMENT_LOCK_SCHEMA_VERSION,
        serde_json::to_value(parsed).unwrap()["schema_version"]
    );
}

#[test]
fn malformed_existing_lock_is_not_replaced() {
    let root = fixture();
    let path = root.path().join(".ayni.lock");
    fs::write(&path, "not-json\n").unwrap();
    let output = lock(&root);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(fs::read_to_string(path).unwrap(), "not-json\n");
}

#[test]
fn identical_checkouts_with_different_directory_names_produce_identical_locks() {
    let first = fixture();
    let second = fixture();
    assert_ne!(first.path(), second.path());
    assert!(lock(&first).status.success());
    assert!(lock(&second).status.success());
    assert_eq!(
        fs::read(first.path().join(".ayni.lock")).unwrap(),
        fs::read(second.path().join(".ayni.lock")).unwrap()
    );
}

#[cfg(unix)]
fn fake_mise(root: &TempDir, body: &str) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let bin = root.path().join("bin");
    fs::create_dir_all(&bin).unwrap();
    let executable = bin.join("mise");
    fs::write(
        &executable,
        format!(
            "#!/bin/sh\nwhile [ \"$1\" = \"--no-config\" ] || [ \"$1\" = \"--no-env\" ] || [ \"$1\" = \"--no-hooks\" ]; do shift; done\n{body}\n"
        ),
    )
    .unwrap();
    let mut permissions = fs::metadata(&executable).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(executable, permissions).unwrap();
    let docker = bin.join("docker");
    fs::write(
        &docker,
        "#!/bin/sh\nif [ \"$1\" = \"buildx\" ]; then printf '{\"digest\":\"sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\"}\\n'; exit 0; fi\nexit 1\n",
    )
    .unwrap();
    let mut permissions = fs::metadata(&docker).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(docker, permissions).unwrap();
    bin
}

#[cfg(unix)]
#[test]
fn node_range_resolution_selects_the_highest_matching_exact_version() {
    let root = TempDir::new().expect("tempdir");
    fs::write(root.path().join(".ayni.toml"), "[checks]\ntest = false\ncoverage = false\nsize = false\ncomplexity = false\ndeps = false\nmutation = false\n[languages]\nenabled = [\"node\"]\n").unwrap();
    fs::write(
        root.path().join("package.json"),
        r#"{"name":"fixture","engines":{"node":">=20 <23"},"packageManager":"npm@10.8.0"}"#,
    )
    .unwrap();
    fs::write(
        root.path().join("package-lock.json"),
        r#"{"name":"fixture","lockfileVersion":3,"packages":{"":{"name":"fixture"}}}"#,
    )
    .unwrap();
    let bin = fake_mise(
        &root,
        r#"if [ "$1" = "version" ]; then echo "2026.8.7 linux-x64"; exit 0; fi
if [ "$1" = "ls-remote" ] && [ "$2" = "node" ]; then printf '18.20.0\n20.15.1\n22.12.0\n23.1.0\n'; exit 0; fi
exit 1"#,
    );
    let inherited = std::env::var_os("PATH").unwrap_or_default();
    let path = std::env::join_paths(std::iter::once(bin).chain(std::env::split_paths(&inherited)))
        .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_ayni"))
        .args(["env", "lock", "--repo-root"])
        .arg(root.path())
        .env("PATH", path)
        .output()
        .expect("launch ayni");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let lock: serde_json::Value =
        serde_json::from_slice(&fs::read(root.path().join(".ayni.lock")).unwrap()).unwrap();
    assert_eq!(lock["targets"][0]["runtimes"][0]["version"], "22.12.0");
}

#[cfg(unix)]
#[test]
fn pnpm_project_tools_resolve_from_the_native_lockfile() {
    let root = TempDir::new().expect("tempdir");
    fs::write(root.path().join(".ayni.toml"), "[checks]\ntest = true\ncoverage = false\nsize = false\ncomplexity = false\ndeps = false\nmutation = false\n[languages]\nenabled = [\"node\"]\n").unwrap();
    fs::write(root.path().join(".node-version"), "24.14.0\n").unwrap();
    fs::write(
        root.path().join("package.json"),
        r#"{"name":"fixture","packageManager":"pnpm@11.15.1","devDependencies":{"vitest":"^3.2.0"}}"#,
    )
    .unwrap();
    fs::write(
        root.path().join("pnpm-lock.yaml"),
        r#"lockfileVersion: '9.0'
importers:
  .:
    devDependencies:
      vitest:
        specifier: ^3.2.0
        version: 3.2.4
packages:
  vitest@3.2.4: {}
"#,
    )
    .unwrap();
    fake_mise(
        &root,
        r#"if [ "$1" = "version" ]; then echo "2026.8.7 linux-x64"; exit 0; fi
exit 1"#,
    );
    let output = lock(&root);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let parsed: ayni_core::EnvironmentLock =
        serde_json::from_slice(&fs::read(root.path().join(".ayni.lock")).unwrap()).unwrap();
    let target = &parsed.targets()[0];
    assert_eq!(target.package_manager.as_ref().unwrap().family, "pnpm");
    assert_eq!(target.signal_tools[0].tool, "vitest");
    assert_eq!(target.signal_tools[0].version, "3.2.4");
}

#[cfg(unix)]
#[test]
fn provider_failure_uses_execution_exit_code_and_preserves_existing_bytes() {
    let root = fixture();
    assert!(lock(&root).status.success());
    let existing = fs::read(root.path().join(".ayni.lock")).unwrap();
    fs::write(
        root.path().join("rust-toolchain.toml"),
        "[toolchain]\nchannel = \"stable\"\n",
    )
    .unwrap();
    let bin = fake_mise(
        &root,
        r#"if [ "$1" = "version" ]; then echo "2026.8.7 linux-x64"; exit 0; fi
exit 1"#,
    );
    let inherited = std::env::var_os("PATH").unwrap_or_default();
    let path = std::env::join_paths(std::iter::once(bin).chain(std::env::split_paths(&inherited)))
        .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_ayni"))
        .args(["env", "lock", "--repo-root"])
        .arg(root.path())
        .env("PATH", path)
        .output()
        .expect("launch ayni");
    assert_eq!(output.status.code(), Some(4));
    assert_eq!(fs::read(root.path().join(".ayni.lock")).unwrap(), existing);
}

#[cfg(unix)]
#[test]
fn unresolved_rust_catalog_tool_is_resolved_to_an_exact_version() {
    let root = fixture();
    fs::write(root.path().join(".ayni.toml"), "[checks]\ntest = false\ncoverage = false\nsize = false\ncomplexity = true\ndeps = false\nmutation = false\n[languages]\nenabled = [\"rust\"]\n[rust.complexity]\nfn_cyclomatic = { warn = 10, fail = 15 }\n").unwrap();
    let bin = fake_mise(
        &root,
        r#"if [ "$1" = "version" ]; then echo "2026.8.7 linux-x64"; exit 0; fi
if [ "$1" = "latest" ] && [ "$2" = "rust@1.93.0" ]; then echo "1.93.0"; exit 0; fi
if [ "$1" = "latest" ] && [ "$2" = "cargo:rust-code-analysis-cli" ]; then echo "0.6.19"; exit 0; fi
exit 1"#,
    );
    let inherited = std::env::var_os("PATH").unwrap_or_default();
    let path = std::env::join_paths(std::iter::once(bin).chain(std::env::split_paths(&inherited)))
        .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_ayni"))
        .args(["env", "lock", "--repo-root"])
        .arg(root.path())
        .env("PATH", path)
        .output()
        .expect("launch ayni");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let lock: serde_json::Value =
        serde_json::from_slice(&fs::read(root.path().join(".ayni.lock")).unwrap()).unwrap();
    assert_eq!(
        lock["targets"][0]["signal_tools"][0]["tool"],
        "rust-code-analysis-cli"
    );
    assert_eq!(lock["targets"][0]["signal_tools"][0]["version"], "0.6.19");
}

#[test]
fn node_workspace_uses_non_hoisted_package_lock_tool_resolution() {
    let root = TempDir::new().expect("tempdir");
    fs::create_dir_all(root.path().join("apps/web")).unwrap();
    fs::write(root.path().join(".ayni.toml"), "[checks]\ntest = true\ncoverage = false\nsize = false\ncomplexity = false\ndeps = false\nmutation = false\n[languages]\nenabled = [\"node\"]\n[node]\nroots = [\"apps/web\"]\n").unwrap();
    fs::write(
        root.path().join("package.json"),
        r#"{"name":"workspace","private":true,"workspaces":["apps/*"],"packageManager":"npm@10.8.0"}"#,
    )
    .unwrap();
    fs::write(
        root.path().join("apps/web/package.json"),
        r#"{"name":"web","engines":{"node":"22.12.0"},"devDependencies":{"vitest":"3.2.4"}}"#,
    )
    .unwrap();
    fs::write(
        root.path().join("package-lock.json"),
        r#"{"name":"workspace","lockfileVersion":3,"packages":{"":{"name":"workspace"},"apps/web":{"name":"web","devDependencies":{"vitest":"3.2.4"}},"apps/web/node_modules/vitest":{"version":"3.2.4"}}}"#,
    )
    .unwrap();
    fake_mise(
        &root,
        r#"if [ "$1" = "version" ]; then echo "2026.8.7 linux-x64"; exit 0; fi
if [ "$1" = "ls-remote" ] && [ "$2" = "node" ]; then echo "22.12.0"; exit 0; fi
exit 1"#,
    );
    let output = lock(&root);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let lock: serde_json::Value =
        serde_json::from_slice(&fs::read(root.path().join(".ayni.lock")).unwrap()).unwrap();
    assert_eq!(lock["targets"][0]["target"]["root"], "apps/web");
    assert_eq!(lock["targets"][0]["signal_tools"][0]["tool"], "vitest");
    assert_eq!(lock["targets"][0]["signal_tools"][0]["version"], "3.2.4");
}

#[test]
fn source_content_change_updates_lock_even_when_resolved_version_is_unchanged() {
    let root = fixture();
    assert!(lock(&root).status.success());
    let path = root.path().join(".ayni.lock");
    let before = fs::read(&path).unwrap();
    fs::write(
        root.path().join("rust-toolchain.toml"),
        "# same resolved toolchain\n[toolchain]\nchannel = \"1.93.0\"\n",
    )
    .unwrap();
    let output = lock(&root);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_ne!(before, fs::read(path).unwrap());
}

#[test]
fn concurrent_source_change_fails_without_replacing_existing_lock() {
    let root = fixture();
    assert!(lock(&root).status.success());
    let path = root.path().join(".ayni.lock");
    let existing = fs::read(&path).unwrap();
    fs::write(
        root.path().join("rust-toolchain.toml"),
        "[toolchain]\nchannel = \"stable\"\n",
    )
    .unwrap();
    fake_mise(
        &root,
        r#"if [ "$1" = "latest" ] && [ "$2" = "rust@stable" ]; then
  printf '# changed while locking\n[toolchain]\nchannel = "stable"\n' > rust-toolchain.toml
  echo "1.94.0"
  exit 0
fi
if [ "$1" = "version" ]; then echo "2026.8.7 linux-x64"; exit 0; fi
exit 1"#,
    );
    let output = lock(&root);
    assert_eq!(output.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&output.stderr).contains("changed during locking"));
    assert_eq!(fs::read(path).unwrap(), existing);
}

#[test]
fn go_python_and_kotlin_exact_requirements_lock_without_language_specific_cli_logic() {
    let root = TempDir::new().expect("tempdir");
    fs::write(
        root.path().join(".ayni.toml"),
        "[checks]\ntest=true\ncoverage=false\nsize=false\ncomplexity=false\ndeps=false\nmutation=false\n[languages]\nenabled=[\"go\",\"python\",\"kotlin\"]\n[go]\nroots=[\"go\"]\n[python]\nroots=[\"python\"]\n[kotlin]\nroots=[\"kotlin\"]\n",
    )
    .unwrap();

    fs::create_dir(root.path().join("go")).unwrap();
    fs::write(
        root.path().join("go/go.mod"),
        "module example.com/go\n\ngo 1.23\ntoolchain go1.23.4\n",
    )
    .unwrap();

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

    fake_mise(
        &root,
        r#"if [ "$1" = "version" ]; then echo "2026.8.7 linux-x64"; exit 0; fi
exit 1"#,
    );
    let output = lock(&root);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let lock: serde_json::Value =
        serde_json::from_slice(&fs::read(root.path().join(".ayni.lock")).unwrap()).unwrap();
    assert_eq!(lock["targets"].as_array().unwrap().len(), 3);
    assert_eq!(lock["targets"][0]["target"]["language"], "go");
    assert_eq!(lock["targets"][1]["target"]["language"], "python");
    assert_eq!(lock["targets"][2]["target"]["language"], "kotlin");
    assert_eq!(lock["targets"][2]["runtimes"][0]["version"], "21.0.6");
}

#[cfg(unix)]
#[test]
fn python_requires_range_selects_highest_matching_cpython() {
    let root = TempDir::new().expect("tempdir");
    fs::write(
        root.path().join(".ayni.toml"),
        "[checks]\ntest=false\ncoverage=false\nsize=false\ncomplexity=false\ndeps=false\nmutation=false\n[languages]\nenabled=[\"python\"]\n",
    )
    .unwrap();
    fs::write(
        root.path().join("pyproject.toml"),
        "[project]\nname='fixture'\nrequires-python='>=3.11,<3.13'\n[tool.uv]\nrequired-version='0.6.0'\n",
    )
    .unwrap();
    fs::write(root.path().join("uv.lock"), "version=1\n").unwrap();
    fake_mise(
        &root,
        r#"if [ "$1" = "version" ]; then echo "2026.8.7 linux-x64"; exit 0; fi
if [ "$1" = "ls-remote" ] && [ "$2" = "python" ]; then printf '3.10.14\n3.11.9\n3.12.8\n3.13.1\n'; exit 0; fi
exit 1"#,
    );
    let output = lock(&root);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let lock: serde_json::Value =
        serde_json::from_slice(&fs::read(root.path().join(".ayni.lock")).unwrap()).unwrap();
    assert_eq!(lock["targets"][0]["runtimes"][0]["version"], "3.12.8");
}
