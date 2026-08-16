use std::fs;
use std::process::Command;
use tempfile::TempDir;

fn command(config: &std::path::Path, output: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_ayni"))
        .args(["contract", "validate", "--config"])
        .arg(config)
        .args(["--output", output])
        .output()
        .expect("launch ayni")
}

#[test]
fn validate_emits_concise_human_success_and_the_contract_json_projection() {
    let root = TempDir::new().expect("fixture");
    let config = root.path().join(".ayni.toml");
    fs::write(&config, "[languages]\nenabled = [\"rust\"]\n").expect("config");

    let human = command(&config, "human");
    assert!(human.status.success());
    assert_eq!(human.stdout, b"contract valid\n");
    assert!(human.stderr.is_empty());

    let json = command(&config, "json");
    assert!(json.status.success());
    assert!(json.stderr.is_empty());
    let value: serde_json::Value = serde_json::from_slice(&json.stdout).expect("projection");
    assert_eq!(value["projection_version"], "0.1.0");
    assert_eq!(value["languages"][0]["language"], "rust");
}
