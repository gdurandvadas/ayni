use std::process::Command;

fn ayni() -> Command {
    Command::new(env!("CARGO_BIN_EXE_ayni"))
}

#[test]
fn unavailable_greenfield_operations_use_incomplete_exit_and_clean_stdout() {
    for arguments in [["env", "doctor"], ["env", "build"], ["env", "shell"]] {
        let output = ayni().args(arguments).output().expect("launch ayni");
        assert_eq!(output.status.code(), Some(4));
        assert!(output.stdout.is_empty());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("is not implemented yet"),
            "stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn managed_quality_commands_use_environment_exit_and_suggest_host() {
    for arguments in [vec!["check"], vec!["verify", "test"]] {
        let output = ayni().args(&arguments).output().expect("launch ayni");
        assert_eq!(output.status.code(), Some(3), "{arguments:?}");
        assert!(output.stdout.is_empty(), "{arguments:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("rerun with --host"),
            "{arguments:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn invalid_and_superseded_commands_use_cli_input_exit() {
    for command in ["analyze", "install", "artifact"] {
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
