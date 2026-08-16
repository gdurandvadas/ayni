use std::process::Command;

fn ayni() -> Command {
    Command::new(env!("CARGO_BIN_EXE_ayni"))
}

#[test]
fn environment_operations_require_a_valid_lock_without_implicit_provisioning() {
    for arguments in [["env", "doctor"], ["env", "build"], ["env", "shell"]] {
        let output = ayni().args(arguments).output().expect("launch ayni");
        assert_eq!(output.status.code(), Some(3));
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8_lossy(&output.stderr).contains("environment lock"));
    }
}

#[test]
fn managed_check_and_verify_require_a_lock_without_implicit_provisioning() {
    let check = ayni().arg("check").output().expect("launch ayni");
    assert_eq!(check.status.code(), Some(3));
    assert!(check.stdout.is_empty());
    assert!(String::from_utf8_lossy(&check.stderr).contains("environment lock"));

    let verify = ayni()
        .args(["verify", "test"])
        .output()
        .expect("launch ayni");
    assert_eq!(verify.status.code(), Some(3));
    assert!(verify.stdout.is_empty());
    assert!(String::from_utf8_lossy(&verify.stderr).contains("environment lock"));
}

#[test]
fn invalid_and_superseded_commands_use_cli_input_exit() {
    for command in ["analyze", "install", "artifact", "init"] {
        let output = ayni().arg(command).output().expect("launch ayni");
        assert_eq!(output.status.code(), Some(2), "{command}");
        assert!(output.stdout.is_empty(), "{command}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("unrecognized subcommand"),
            "{command}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn invalid_host_contract_uses_contract_exit() {
    let output = ayni()
        .args(["check", "--host", "--config", "missing.toml"])
        .output()
        .expect("launch ayni");

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("failed to read"));
}

#[test]
fn version_flag_remains_successful_without_a_command_alias() {
    let output = ayni().arg("--version").output().expect("launch ayni");

    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    assert!(String::from_utf8_lossy(&output.stdout).starts_with("ayni "));
}

#[test]
fn unfinished_results_show_is_not_publicly_available() {
    let output = ayni()
        .args(["results", "show", "--file", "result.json"])
        .output()
        .expect("launch ayni");

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("unrecognized subcommand"));
}
