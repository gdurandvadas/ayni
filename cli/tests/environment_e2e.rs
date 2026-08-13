use serde_json::Value;
use std::fs;
use std::process::{Command, Output};
use tempfile::TempDir;

fn show(root: &TempDir, output: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ayni"))
        .args(["env", "show", "--repo-root"])
        .arg(root.path())
        .args(["--output", output])
        .output()
        .expect("launch ayni")
}

#[test]
fn mixed_rust_and_node_json_is_deterministic_and_read_only() {
    let root = TempDir::new().expect("tempdir");
    fs::write(root.path().join(".ayni.toml"), "[checks]\ntest = true\n[languages]\nenabled = [\"node\", \"rust\"]\n[rust]\nroots = [\"rust\"]\n[node]\nroots = [\"node\"]\n").unwrap();
    fs::create_dir(root.path().join("rust")).unwrap();
    fs::write(
        root.path().join("rust/Cargo.toml"),
        "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .unwrap();
    fs::create_dir(root.path().join("node")).unwrap();
    fs::write(
        root.path().join("node/package.json"),
        "{\"name\":\"fixture\",\"engines\":{\"node\":\">=20\"}}",
    )
    .unwrap();

    let first = show(&root, "json");
    let second = show(&root, "json");
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert_eq!(first.stdout, second.stdout);
    assert!(first.stderr.is_empty());
    assert!(
        !root.path().join(".ayni").exists(),
        "show must not write artifacts"
    );
    let plan: Value = serde_json::from_slice(&first.stdout).expect("one JSON plan");
    assert_eq!(
        plan["repository"]["name"],
        root.path().file_name().unwrap().to_str().unwrap()
    );
    assert_eq!(plan["platforms"].as_array().unwrap().len(), 2);
    assert_eq!(plan["targets"].as_array().unwrap().len(), 2);
    assert_eq!(plan["targets"][0]["target"]["language"], "rust");
    assert_eq!(plan["targets"][1]["target"]["language"], "node");
}

#[test]
fn conflicts_remain_visible_and_successful() {
    let root = TempDir::new().expect("tempdir");
    fs::write(
        root.path().join(".ayni.toml"),
        "[languages]\nenabled = [\"node\"]\n",
    )
    .unwrap();
    fs::write(root.path().join("package.json"), "{\"name\":\"fixture\"}").unwrap();
    fs::write(root.path().join(".node-version"), "20.0.0\n").unwrap();
    fs::write(root.path().join(".nvmrc"), "22.0.0\n").unwrap();
    let output = show(&root, "json");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let plan: Value = serde_json::from_slice(&output.stdout).expect("JSON plan");
    assert!(!plan["conflicts"].as_array().unwrap().is_empty());
}

#[test]
fn human_output_explains_sources_tools_locks_and_conflicts() {
    let root = TempDir::new().expect("tempdir");
    fs::write(
        root.path().join(".ayni.toml"),
        "[checks]\ntest = true\ncoverage = false\nsize = false\ncomplexity = false\ndeps = false\nmutation = false\n[languages]\nenabled = [\"node\"]\n",
    )
    .unwrap();
    fs::write(
        root.path().join("package.json"),
        r#"{"name":"fixture","packageManager":"npm@10.0.0","engines":{"node":"22.0.0"},"devDependencies":{"vitest":"3.0.0"}}"#,
    )
    .unwrap();
    fs::write(root.path().join("package-lock.json"), "{}").unwrap();
    let output = show(&root, "human");
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("package: fixture"));
    assert!(stdout.contains("package_json_engines_node"));
    assert!(stdout.contains("tool: vitest"));
    assert!(stdout.contains("dependency lock: package-lock.json sha256:"));
}

#[test]
fn undetected_configured_target_uses_environment_exit() {
    let root = TempDir::new().expect("tempdir");
    fs::write(
        root.path().join(".ayni.toml"),
        "[languages]\nenabled = [\"rust\"]\n[rust]\nroots = [\"missing\"]\n",
    )
    .unwrap();
    let output = show(&root, "json");
    assert_eq!(output.status.code(), Some(3));
    assert!(output.stdout.is_empty());
}

#[test]
fn configured_tool_override_fails_instead_of_planning_the_default_tool() {
    let root = TempDir::new().expect("tempdir");
    fs::write(
        root.path().join(".ayni.toml"),
        "[checks]\ntest = true\ncoverage = false\nsize = false\ncomplexity = false\ndeps = false\nmutation = false\n[languages]\nenabled = [\"rust\"]\n[rust.tooling.test]\ncommand = \"nextest\"\n",
    )
    .unwrap();
    fs::write(
        root.path().join("Cargo.toml"),
        "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    let output = show(&root, "json");
    assert_eq!(output.status.code(), Some(3));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("tool override"));
}

#[test]
fn malformed_configuration_fails_explicitly() {
    let root = TempDir::new().expect("tempdir");
    fs::write(root.path().join(".ayni.toml"), "not = [valid").unwrap();
    let output = show(&root, "human");
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("failed to load environment configuration")
    );
}

#[test]
fn unsupported_environment_adapter_fails_explicitly() {
    let root = TempDir::new().expect("tempdir");
    fs::write(
        root.path().join(".ayni.toml"),
        "[languages]\nenabled = [\"go\"]\n",
    )
    .unwrap();
    fs::write(root.path().join("go.mod"), "module fixture\n\ngo 1.23\n").unwrap();
    let output = show(&root, "human");
    assert_eq!(output.status.code(), Some(3));
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("environment discovery capability is unsupported")
    );
}
