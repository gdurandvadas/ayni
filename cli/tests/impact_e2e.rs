use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;

struct Fixture {
    _temp: TempDir,
    root: PathBuf,
    config: PathBuf,
    base: String,
}

impl Fixture {
    fn new() -> Self {
        let temp = TempDir::new().expect("fixture");
        let root = temp.path().to_path_buf();
        let runner = root.join("test-runner.sh");
        fs::write(
            &runner,
            "#!/bin/sh\nprintf 'test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\\n'\n",
        )
        .expect("runner");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&runner, fs::Permissions::from_mode(0o755)).expect("permissions");
        }
        fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers = [\"crates/base\", \"crates/app\"]\nresolver = \"2\"\n",
        )
        .expect("workspace");
        for package in ["base", "app"] {
            let dir = root.join("crates").join(package);
            fs::create_dir_all(dir.join("src")).expect("source dir");
            let dependencies = if package == "app" {
                "\n[dependencies]\nbase = { path = \"../base\" }\n"
            } else {
                ""
            };
            fs::write(
                dir.join("Cargo.toml"),
                format!(
                    "[package]\nname = \"{package}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n{dependencies}"
                ),
            )
            .expect("manifest");
            fs::write(dir.join("src/lib.rs"), "pub fn value() -> u8 { 1 }\n").expect("source");
        }
        fs::write(root.join(".gitignore"), ".ayni/\n").expect("gitignore");
        let config = root.join(".ayni.toml");
        fs::write(
            &config,
            format!(
                r#"[checks]
test = true
coverage = false
size = false
complexity = false
deps = false
mutation = false

[languages]
enabled = ["rust"]

[rust]
roots = ["."]

[rust.tooling.test]
command = {}
"#,
                toml_string(&runner)
            ),
        )
        .expect("config");
        git(&root, &["init", "-q"]);
        git(&root, &["config", "user.name", "Ayni Test"]);
        git(&root, &["config", "user.email", "ayni@example.invalid"]);
        git(&root, &["config", "commit.gpgsign", "false"]);
        git(&root, &["add", "."]);
        git(&root, &["commit", "-qm", "base"]);
        let base = git_output(&root, &["rev-parse", "HEAD"]).trim().to_owned();
        fs::write(
            root.join("crates/base/src/lib.rs"),
            "pub fn value() -> u8 { 2 }\n",
        )
        .expect("change");
        Self {
            _temp: temp,
            root,
            config,
            base,
        }
    }

    fn command(&self, subcommand: &str) -> Command {
        self.command_with_config(subcommand, self.config.to_str().expect("config"))
    }

    fn command_with_config(&self, subcommand: &str, config: &str) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_ayni"));
        command.current_dir(&self.root).args(["impact", subcommand]);
        if subcommand == "run" {
            command.arg("--host");
        }
        command.args(["--base", &self.base, "--config", config, "--output", "json"]);
        command
    }
}

fn toml_string(path: &Path) -> String {
    format!("{:?}", path.to_string_lossy())
}

fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .expect("git");
    assert!(
        output.status.success(),
        "git {}: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_output(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .expect("git");
    assert!(output.status.success());
    String::from_utf8(output.stdout).expect("utf8")
}

fn run(mut command: Command) -> Output {
    command.output().expect("ayni")
}

#[test]
fn show_explains_reverse_dependency_impact_without_writing_artifacts() {
    let fixture = Fixture::new();
    let output = run(fixture.command("show"));
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let document: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(document["schema_version"], "0.1.0");
    assert_eq!(document["plan"]["repository_completion_required"], true);
    assert_eq!(document["plan"]["candidate"]["kind"], "working_tree");
    let checks = document["plan"]["selected_checks"]
        .as_array()
        .expect("checks");
    let packages = checks
        .iter()
        .filter_map(|check| check["package"].as_str())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(packages, std::collections::BTreeSet::from(["app", "base"]));
    assert!(checks.iter().any(|check| {
        check["reasons"].as_array().is_some_and(|reasons| {
            reasons
                .iter()
                .any(|reason| reason["kind"] == "reverse_dependency")
        })
    }));
    assert!(!fixture.root.join(".ayni/impact").exists());
}

#[test]
fn show_resolves_relative_and_absolute_config_paths_to_the_same_plan() {
    let fixture = Fixture::new();
    let relative = run(fixture.command_with_config("show", "./.ayni.toml"));
    assert!(
        relative.status.success(),
        "{}",
        String::from_utf8_lossy(&relative.stderr)
    );
    let relative: Value = serde_json::from_slice(&relative.stdout).expect("relative plan");

    let canonical_config = fixture.config.canonicalize().expect("canonical config");
    let absolute =
        run(fixture
            .command_with_config("show", canonical_config.to_str().expect("absolute config")));
    assert!(
        absolute.status.success(),
        "{}",
        String::from_utf8_lossy(&absolute.stderr)
    );
    let absolute: Value = serde_json::from_slice(&absolute.stdout).expect("absolute plan");

    assert_eq!(relative, absolute);
    assert!(!fixture.root.join(".ayni/impact").exists());
}

#[test]
fn show_captures_staged_unstaged_untracked_and_renamed_changes() {
    let fixture = Fixture::new();
    git(&fixture.root, &["add", "crates/base/src/lib.rs"]);
    fs::write(
        fixture.root.join("crates/base/src/lib.rs"),
        "pub fn value() -> u8 { 3 }\n",
    )
    .expect("unstaged change");
    fs::write(
        fixture.root.join("crates/base/src/new.rs"),
        "pub fn added() {}\n",
    )
    .expect("untracked change");
    git(
        &fixture.root,
        &["mv", "crates/app/src/lib.rs", "crates/app/src/main.rs"],
    );

    let output = run(fixture.command("show"));

    assert!(output.status.success());
    let document: Value = serde_json::from_slice(&output.stdout).expect("json");
    let changes = document["plan"]["changes"].as_array().expect("changes");
    assert!(changes.iter().any(|change| {
        change["kind"] == "modified" && change["path"] == "crates/base/src/lib.rs"
    }));
    assert!(
        changes.iter().any(|change| {
            change["kind"] == "added" && change["path"] == "crates/base/src/new.rs"
        })
    );
    assert!(changes.iter().any(|change| {
        change["kind"] == "renamed"
            && change["path"] == "crates/app/src/main.rs"
            && change["previous_path"] == "crates/app/src/lib.rs"
    }));
}

#[test]
fn run_persists_separate_non_completion_evidence_and_preserves_other_slots() {
    let fixture = Fixture::new();
    fs::create_dir_all(fixture.root.join(".ayni/last")).expect("last");
    fs::create_dir_all(fixture.root.join(".ayni/verify/last")).expect("verify");
    fs::write(
        fixture.root.join(".ayni/last/signals.json"),
        "check sentinel",
    )
    .expect("check sentinel");
    fs::write(
        fixture.root.join(".ayni/verify/last/signals.json"),
        "verify sentinel",
    )
    .expect("verify sentinel");

    let output = run(fixture.command("run"));
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout");
    let document: Value = serde_json::from_str(&stdout).expect("json");
    assert_eq!(document["schema_version"], "0.1.0");
    assert_eq!(document["signal_schema_version"], "0.4.0");
    assert_eq!(document["repository_completion"]["evaluated"], false);
    assert_eq!(
        document["repository_completion"]["required_command"],
        "ayni check"
    );
    assert_eq!(document["execution"]["state"], "complete");
    assert_eq!(document["execution"]["planned_jobs"], 2);
    assert_eq!(document["rows"].as_array().expect("rows").len(), 2);
    assert_eq!(
        fs::read_to_string(fixture.root.join(".ayni/impact/last/impact.json"))
            .expect("impact artifact"),
        stdout
    );
    assert_eq!(
        fs::read_to_string(fixture.root.join(".ayni/last/signals.json")).expect("check"),
        "check sentinel"
    );
    assert_eq!(
        fs::read_to_string(fixture.root.join(".ayni/verify/last/signals.json")).expect("verify"),
        "verify sentinel"
    );
}

#[test]
fn run_fails_closed_when_the_candidate_changes_during_execution() {
    let fixture = Fixture::new();
    fs::write(
        fixture.root.join("test-runner.sh"),
        "#!/bin/sh\nprintf '\\n// drift' >> crates/base/src/lib.rs\nprintf 'test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\\n'\n",
    )
    .expect("drifting runner");

    let output = run(fixture.command("run"));

    assert_eq!(output.status.code(), Some(4));
    let document: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(document["execution"]["state"], "incomplete");
    assert!(
        document["execution"]["issues"]
            .as_array()
            .expect("issues")
            .iter()
            .any(|issue| issue["message"]
                .as_str()
                .is_some_and(|message| message.contains("candidate changed")))
    );
}

#[test]
fn planning_rejects_a_repository_that_does_not_ignore_generated_evidence() {
    let fixture = Fixture::new();
    fs::write(fixture.root.join(".gitignore"), "target/\n").expect("remove ignore");

    let output = run(fixture.command("show"));

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains(".ayni/ must be ignored"));
}

#[test]
fn contract_change_broadens_and_invalid_base_is_input_failure() {
    let fixture = Fixture::new();
    let mut config = fs::read_to_string(&fixture.config).expect("config");
    config.push('\n');
    fs::write(&fixture.config, config).expect("change config");
    let output = run(fixture.command("show"));
    assert!(output.status.success());
    let document: Value = serde_json::from_slice(&output.stdout).expect("json");
    let checks = document["plan"]["selected_checks"]
        .as_array()
        .expect("checks");
    assert_eq!(checks.len(), 1);
    assert!(checks[0].get("package").is_none());
    assert_eq!(checks[0]["reasons"][0]["kind"], "contract_changed");

    let invalid = fixture.command("show");
    let args = invalid
        .get_args()
        .map(|value| value.to_owned())
        .collect::<Vec<_>>();
    let position = args
        .iter()
        .position(|value| value == "--base")
        .expect("base")
        + 1;
    let mut command = Command::new(env!("CARGO_BIN_EXE_ayni"));
    command.current_dir(&fixture.root);
    for (index, arg) in args.into_iter().enumerate() {
        command.arg(if index == position {
            std::ffi::OsString::from("does-not-exist")
        } else {
            arg
        });
    }
    let output = run(command);
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("git rev-parse"));
}
