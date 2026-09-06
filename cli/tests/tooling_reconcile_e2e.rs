use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
use tempfile::TempDir;
fn fixture(language: &str, ownership: &str, signal: &str) -> TempDir {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join(".ayni.toml"), format!("[checks]\ntest = {}\ncoverage = {}\nsize = false\ncomplexity = false\ndeps = false\nmutation = false\n[languages]\nenabled = [\"{language}\"]\n[{language}]\nroots = [\".\"]\n[environment.signal_tools]\nownership = \"{ownership}\"\n", signal == "test", signal == "coverage")).unwrap();
    dir
}
fn run(root: &Path, check: bool) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_ayni"));
    command
        .args(["tools", "reconcile", "--output", "json", "--repo-root"])
        .arg(root);
    if check {
        command.arg("--check");
    }
    command.output().unwrap()
}
fn json(output: &Output) -> serde_json::Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|e| panic!("{e}: {:?}", output))
}
#[test]
fn missing_node_tools_are_a_successful_read_only_preview_and_failed_check() {
    let dir = fixture("node", "ayni", "test");
    let manifest = r#"{"name":"fixture","version":"1.0.0","packageManager":"npm@10.8.2"}"#;
    fs::write(dir.path().join("package.json"), manifest).unwrap();
    let first = run(dir.path(), false);
    assert!(first.status.success(), "{first:?}");
    assert_eq!(first.stdout, run(dir.path(), false).stdout);
    let value = json(&first);
    assert_eq!(value["projection_version"], "0.1.0");
    assert_eq!(value["ownership"], "ayni");
    assert_eq!(value["reconciliation_required"], true);
    assert_eq!(value["targets"][0]["tools"][0]["tool"], "vitest");
    assert_eq!(run(dir.path(), true).status.code(), Some(1));
    assert_eq!(
        fs::read_to_string(dir.path().join("package.json")).unwrap(),
        manifest
    );
    assert!(!dir.path().join(".ayni").exists());
    assert!(!dir.path().join("package-lock.json").exists());
}
#[test]
fn project_versions_are_authoritative_but_ayni_requires_baseline() {
    for (ownership, required) in [("project", false), ("ayni", true)] {
        let dir = fixture("node", ownership, "test");
        fs::write(
            dir.path().join("package.json"),
            r#"{"name":"fixture","devDependencies":{"vitest":"^2.0.0"}}"#,
        )
        .unwrap();
        fs::write(
            dir.path().join("package-lock.json"),
            r#"{"lockfileVersion":3,"packages":{"node_modules/vitest":{"version":"2.1.0"}}}"#,
        )
        .unwrap();
        let result = run(dir.path(), false);
        assert!(result.status.success(), "{result:?}");
        assert_eq!(json(&result)["reconciliation_required"], required);
        assert_eq!(run(dir.path(), true).status.success(), !required);
    }
}
#[test]
fn node_incompatible_lock_fails_project_check() {
    let dir = fixture("node", "project", "test");
    fs::write(
        dir.path().join("package.json"),
        r#"{"devDependencies":{"vitest":"^3.0.0"}}"#,
    )
    .unwrap();
    fs::write(
        dir.path().join("package-lock.json"),
        r#"{"packages":{"node_modules/vitest":{"version":"2.1.0"}}}"#,
    )
    .unwrap();
    assert_eq!(run(dir.path(), true).status.code(), Some(1));
}
#[test]
fn python_missing_lock_still_reports_every_enabled_baseline() {
    let dir = fixture("python", "ayni", "test");
    fs::write(
        dir.path().join("pyproject.toml"),
        "[project]\nname = 'fixture'\nversion = '1.0.0'\n",
    )
    .unwrap();
    let result = run(dir.path(), false);
    assert!(result.status.success(), "{result:?}");
    assert_eq!(
        json(&result)["targets"][0]["tools"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(run(dir.path(), true).status.code(), Some(1));
    assert!(!dir.path().join("uv.lock").exists());
}
#[test]
fn kotlin_preserves_jacoco_and_reports_missing_integrity_evidence() {
    let dir = fixture("kotlin", "ayni", "coverage");
    fs::write(
        dir.path().join("settings.gradle.kts"),
        "rootProject.name = \"fixture\"",
    )
    .unwrap();
    fs::write(
        dir.path().join("build.gradle.kts"),
        "plugins { id(\"jacoco\") }\njacoco { toolVersion = \"0.8.12\" }",
    )
    .unwrap();
    let result = run(dir.path(), false);
    assert!(result.status.success(), "{result:?}");
    let value = json(&result);
    assert_eq!(value["targets"][0]["tools"].as_array().unwrap().len(), 1);
    assert_eq!(value["targets"][0]["tools"][0]["tool"], "jacoco");
    assert_eq!(run(dir.path(), true).status.code(), Some(1));
}
#[test]
fn custom_overrides_suppress_default_tools() {
    let dir = fixture("node", "ayni", "test");
    let path = dir.path().join(".ayni.toml");
    let mut config = fs::read_to_string(&path).unwrap();
    config.push_str("\n[node.tooling.test]\ncommand = 'custom-runner'\nargs = []\n");
    fs::write(path, config).unwrap();
    fs::write(dir.path().join("package.json"), "{}").unwrap();
    let result = run(dir.path(), true);
    assert!(result.status.success(), "{result:?}");
    assert!(
        json(&result)["targets"][0]["tools"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
#[test]
fn isolated_tools_require_no_application_manifest_changes() {
    for (language, name, content) in [
        (
            "rust",
            "Cargo.toml",
            "[package]\nname='fixture'\nversion='0.1.0'\n",
        ),
        ("go", "go.mod", "module example.com/fixture\n\ngo 1.24.0\n"),
    ] {
        let dir = fixture(language, "ayni", "coverage");
        fs::write(dir.path().join(name), content).unwrap();
        let result = run(dir.path(), true);
        assert!(result.status.success(), "{language}: {result:?}");
        assert_eq!(fs::read_to_string(dir.path().join(name)).unwrap(), content);
        assert!(
            json(&result)["targets"][0]["edits"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
}
#[cfg(unix)]
#[test]
fn escaping_target_symlink_is_rejected() {
    let dir = fixture("node", "project", "test");
    let outside = TempDir::new().unwrap();
    std::os::unix::fs::symlink(outside.path(), dir.path().join("outside")).unwrap();
    let config = dir.path().join(".ayni.toml");
    fs::write(
        &config,
        fs::read_to_string(&config)
            .unwrap()
            .replace("roots = [\".\"]", "roots = [\"outside\"]"),
    )
    .unwrap();
    assert_eq!(run(dir.path(), false).status.code(), Some(2));
}

#[test]
fn ayni_exact_python_declarations_and_lock_pass_but_drift_and_ambiguity_fail() {
    let dir = fixture("python", "ayni", "test");
    fs::write(dir.path().join("pyproject.toml"), "[project]\nname='fixture'\nversion='1.0.0'\n[dependency-groups]\ndev=['pytest==9.0.3','pytest-json-report==1.5.0']\n").unwrap();
    let lock = "version=1\n[[package]]\nname='pytest'\nversion='9.0.3'\n[[package]]\nname='pytest-json-report'\nversion='1.5.0'\n";
    fs::write(dir.path().join("uv.lock"), lock).unwrap();
    let result = run(dir.path(), true);
    assert!(result.status.success(), "{result:?}");
    fs::write(dir.path().join("uv.lock"), lock.replace("9.0.3", "8.0.0")).unwrap();
    assert_eq!(run(dir.path(), true).status.code(), Some(1));
    fs::write(
        dir.path().join("uv.lock"),
        format!("{lock}\n[[package]]\nname='pytest'\nversion='8.0.0'\n"),
    )
    .unwrap();
    assert_eq!(run(dir.path(), true).status.code(), Some(1));
}
#[test]
fn kotlin_missing_jacoco_version_does_not_select_kover() {
    let dir = fixture("kotlin", "ayni", "coverage");
    fs::write(
        dir.path().join("build.gradle.kts"),
        "plugins { id(\"jacoco\") }",
    )
    .unwrap();
    let value = json(&run(dir.path(), false));
    assert_eq!(value["targets"][0]["tools"][0]["tool"], "jacoco");
    assert_eq!(run(dir.path(), true).status.code(), Some(1));
}
#[test]
fn unsupported_managers_and_plugin_aliases_fail_closed() {
    let dir = fixture("node", "ayni", "test");
    fs::write(
        dir.path().join("package.json"),
        r#"{"packageManager":"yarn@4.0.0"}"#,
    )
    .unwrap();
    assert_eq!(run(dir.path(), true).status.code(), Some(1));
    let dir = fixture("kotlin", "ayni", "coverage");
    fs::write(
        dir.path().join("build.gradle.kts"),
        "plugins { alias(libs.plugins.kover) }",
    )
    .unwrap();
    assert_eq!(run(dir.path(), true).status.code(), Some(1));
}
#[test]
fn a_new_repository_does_not_need_an_ayni_lock_to_reconcile() {
    let dir = fixture("node", "ayni", "test");
    fs::write(
        dir.path().join("package.json"),
        r#"{"devDependencies":{"vitest":"3.2.7"}}"#,
    )
    .unwrap();
    fs::write(
        dir.path().join("package-lock.json"),
        r#"{"packages":{"node_modules/vitest":{"version":"3.2.7"}}}"#,
    )
    .unwrap();
    let output = run(dir.path(), true);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(json(&output)["environment_lock"], "absent");
    fs::write(dir.path().join(".ayni.lock"), "{}").unwrap();
    let output = run(dir.path(), true);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(json(&output)["environment_lock"], "invalid");
}

#[test]
fn current_checkout_lock_is_not_reported_stale_by_digest_encoding() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let output = run(root, true);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(json(&output)["environment_lock"], "recorded_inputs_match");
}
