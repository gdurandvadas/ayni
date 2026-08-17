use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use tempfile::TempDir;

fn ayni() -> Command {
    Command::new(env!("CARGO_BIN_EXE_ayni"))
}

fn display(config: &Path) -> Output {
    ayni()
        .args(["contract", "show", "--config"])
        .arg(config)
        .output()
        .expect("launch ayni binary")
}

fn display_json(config: &Path) -> Output {
    ayni()
        .args(["contract", "show", "--output", "json", "--config"])
        .arg(config)
        .output()
        .expect("launch ayni binary")
}

#[test]
fn displays_deterministic_complete_configured_contract_without_execution() {
    let tempdir = TempDir::new().expect("tempdir");
    let config = tempdir.path().join("policy.toml");
    fs::write(
        &config,
        r#"[checks]
test = true
coverage = true
size = true
complexity = true
deps = true
mutation = false

[environment.tools]
protoc = "35.1"

[environment.debian]
packages = ["libssl-dev"]

[environment.docker]
access = "socket"
network = "bridge"

[languages]
enabled = ["node", "rust", "node"]

[rust]
roots = ["./", "crates\\api//", "crates/api"]

[rust.coverage]
line_percent = { warn = 80, fail = 70 }

[rust.size]
"*.rs" = { warn = 400, fail = 700, exclude = ["target/**", ".ayni/**"] }

[rust.complexity]
fn_cyclomatic = { warn = 10, fail = 20 }

[rust.deps.forbidden]
"core" = ["adapters/*", "cli"]

[rust.tooling.test]
command = "definitely-not-an-installed-command"
args = ["test", "with space"]

[node]
roots = ["apps/web/"]

[node.coverage]
branch_percent = { warn = 75.5, fail = 60 }

[node.complexity]
fn_cognitive = { warn = 8, fail = 12 }

[node.tooling.coverage]
command = "pnpm"
args = ["coverage"]

[node.tooling.mutation]
command = "pnpm"
args = ["mutation"]
"#,
    )
    .expect("write policy");

    let first = display(&config);
    let second = display(&config);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert_eq!(first.stdout, second.stdout, "output must be byte-stable");
    assert!(first.stderr.is_empty());
    assert!(
        !tempdir.path().join(".ayni").exists(),
        "display wrote artifacts"
    );

    let stdout = String::from_utf8(first.stdout).expect("UTF-8 output");
    assert_eq!(stdout.matches("language: node").count(), 1);
    assert_eq!(stdout.matches("language: rust").count(), 1);
    assert!(stdout.find("language: rust").unwrap() < stdout.find("language: node").unwrap());
    for signal in ["test", "coverage", "size", "complexity", "deps", "mutation"] {
        assert_eq!(stdout.matches(&format!("    {signal}:")).count(), 2);
    }
    for expected in [
        "  docker: Socket | network: Bridge",
        "    - protoc@35.1",
        "    - libssl-dev",
        "    - crates/api",
        "line_percent (minimum): warn 80 | fail 70",
        "branch_percent (minimum): not configured",
        "pattern: \"*.rs\" | warn: 400 | fail: 700",
        "exclusions: [\"target/**\", \".ayni/**\"]",
        "fn_cyclomatic (maximum): warn 10 | fail 20",
        "\"core\" -> \"adapters/*\"",
        "tool override: command \"definitely-not-an-installed-command\" | args [\"test\", \"with space\"]",
        "mutation: disabled",
        "rules: not configured",
        "restrictions: not configured",
        "tool override: not configured",
    ] {
        assert!(stdout.contains(expected), "missing {expected:?}:\n{stdout}");
    }
    assert!(
        !stdout.trim_start().starts_with('{'),
        "display must not emit JSON"
    );
    assert!(stdout.contains("projection version 0.2.0"));
    assert!(stdout.contains("warnings:"));
    assert!(stdout.contains("policy.effectiveness.size.no_rules"));
}

#[test]
fn json_display_is_a_deterministic_versioned_projection_with_warnings() {
    let tempdir = TempDir::new().expect("tempdir");
    let config = tempdir.path().join("policy.toml");
    fs::write(
        &config,
        r#"[checks]
test = true
coverage = true
size = true
complexity = true
deps = true
mutation = false

[languages]
enabled = ["rust"]

[rust.complexity]
fn_cognitive = { warn = 10, fail = 20 }
"#,
    )
    .expect("write policy");

    let first = display_json(&config);
    let second = display_json(&config);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert_eq!(first.stdout, second.stdout, "output must be byte-stable");
    assert!(first.stderr.is_empty());
    assert!(
        !tempdir.path().join(".ayni").exists(),
        "display wrote artifacts"
    );

    let value: serde_json::Value = serde_json::from_slice(&first.stdout).expect("JSON projection");
    assert_eq!(value["projection_version"], "0.2.0");
    assert_eq!(value["environment"]["tools"], serde_json::json!([]));
    assert_eq!(
        value["environment"]["debian_packages"],
        serde_json::json!([])
    );
    assert_eq!(value["environment"]["docker"], "none");
    assert_eq!(value["environment"]["network"], "none");
    assert_eq!(value["languages"][0]["language"], "rust");
    assert_eq!(value["languages"][0]["signals"][0]["kind"], "test");
    assert_eq!(
        value["languages"][0]["signals"][1]["detail"]["type"],
        "coverage"
    );
    assert!(
        value["warnings"]
            .as_array()
            .expect("warnings array")
            .iter()
            .any(|warning| {
                warning["code"] == "policy.effectiveness.complexity.missing_required_threshold"
                    && warning["policy_path"] == "rust.complexity.fn_cyclomatic"
            })
    );
}

#[test]
fn policy_loader_errors_are_reported_as_failures() {
    let tempdir = TempDir::new().expect("tempdir");
    let missing = tempdir.path().join("missing.toml");
    let missing_output = display(&missing);
    assert!(!missing_output.status.success());
    assert!(missing_output.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&missing_output.stderr)
            .contains(&format!("failed to read {}", missing.display()))
    );

    let unreadable = tempdir.path().join("directory-not-file");
    fs::create_dir(&unreadable).expect("create directory");
    let unreadable_output = display(&unreadable);
    assert!(!unreadable_output.status.success());
    assert!(unreadable_output.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&unreadable_output.stderr)
            .contains(&format!("failed to read {}", unreadable.display()))
    );

    let malformed = tempdir.path().join("malformed.toml");
    fs::write(&malformed, "this is not = valid TOML").expect("write malformed policy");
    let malformed_output = display(&malformed);
    assert!(!malformed_output.status.success());
    assert!(malformed_output.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&malformed_output.stderr)
            .contains(&format!("failed to parse {}", malformed.display()))
    );

    let invalid = tempdir.path().join("invalid.toml");
    fs::write(
        &invalid,
        "[languages]\nenabled = [\"rust\"]\n[concurrency]\namount = 0\n",
    )
    .expect("write invalid policy");
    let invalid_output = display(&invalid);
    assert!(!invalid_output.status.success());
    assert!(invalid_output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&invalid_output.stderr);
    assert!(stderr.contains(&format!("failed to parse {}", invalid.display())));
    assert!(stderr.contains("concurrency.amount must be at least 1"));
}

#[test]
fn nested_help_exposes_show_and_config_default() {
    let contract_help = ayni()
        .args(["contract", "--help"])
        .output()
        .expect("contract help");
    assert!(contract_help.status.success());
    assert!(String::from_utf8_lossy(&contract_help.stdout).contains("show"));

    let display_help = ayni()
        .args(["contract", "show", "--help"])
        .output()
        .expect("display help");
    assert!(display_help.status.success());
    let stdout = String::from_utf8_lossy(&display_help.stdout);
    assert!(stdout.contains("--config <CONFIG>"));
    assert!(stdout.contains("[default: ./.ayni.toml]"));
    assert!(stdout.contains("--output <OUTPUT>"));
    assert!(stdout.contains("json"));
}
