mod complexity;
mod coverage;
mod deps;
mod size;
pub mod test;

use ayni_adapters_common::collector::{
    CollectorError, CollectorResult, finish_collection, finish_coverage_backed_test,
};
use ayni_core::{
    AdapterError, Language, RunContext, SignalCollector, SignalKind, SignalRow,
    VerificationSelection,
};

#[derive(Debug, Default)]
pub struct RustCollector;

fn cargo_subcommand(args: &[String]) -> Option<&str> {
    let mut index = 0;
    while let Some(arg) = args.get(index).map(String::as_str) {
        if arg == "--" {
            return None;
        }
        if arg.starts_with('+') || arg.starts_with('-') {
            let takes_separate_value =
                matches!(arg, "--color" | "--config" | "--explain" | "-C" | "-Z");
            index += if takes_separate_value { 2 } else { 1 };
            continue;
        }
        return Some(arg);
    }
    None
}

fn cargo_subcommand_executable(kind: SignalKind, program: &str, args: &[String]) -> Option<String> {
    let is_cargo = std::path::Path::new(program)
        .file_stem()
        .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case("cargo"));
    if !is_cargo {
        return None;
    }
    let subcommand = cargo_subcommand(args)
        .filter(|subcommand| matches!(*subcommand, "llvm-cov" | "nextest"))
        .or_else(|| (kind == SignalKind::Coverage && args.is_empty()).then_some("llvm-cov"))?;
    Some(format!("cargo-{subcommand}"))
}

impl SignalCollector for RustCollector {
    fn required_host_executables(&self, kind: SignalKind, context: &RunContext) -> Vec<String> {
        if let Some(command) = context.policy.tool_override_for(Language::Rust, kind) {
            let mut executables = vec![command.command.clone()];
            if let Some(subcommand) =
                cargo_subcommand_executable(kind, &command.command, &command.args)
            {
                executables.push(subcommand);
            }
            return executables;
        }
        match kind {
            SignalKind::Test => vec![String::from("cargo")],
            SignalKind::Coverage => {
                vec![String::from("cargo"), String::from("cargo-llvm-cov")]
            }
            SignalKind::Complexity => {
                let mut commands = vec![String::from("rust-code-analysis-cli")];
                if context.scope.package.is_some() {
                    commands.push(String::from("cargo"));
                }
                commands
            }
            SignalKind::Deps => vec![String::from("cargo")],
            SignalKind::Size | SignalKind::Mutation => Vec::new(),
        }
    }

    fn supports_coverage_backed_test(&self, context: &RunContext) -> bool {
        let tooling = &context.policy.rust.tooling;
        tooling.coverage_satisfies_test
    }

    fn collect_coverage_backed_test(
        &self,
        language: Language,
        context: &RunContext,
        on_line: &mut dyn FnMut(&str),
    ) -> Result<(SignalRow, SignalRow), AdapterError> {
        if language != Language::Rust || !self.supports_coverage_backed_test(context) {
            return Err(AdapterError::new(
                Language::Rust,
                "coverage-backed test collection is not enabled for this Rust target",
            ));
        }
        finish_coverage_backed_test(
            Language::Rust,
            context,
            coverage::collect_with_test_lines(context, on_line),
        )
    }

    fn collect_verification(
        &self,
        kind: SignalKind,
        context: &RunContext,
        selection: &VerificationSelection,
        on_line: &mut dyn FnMut(&str),
    ) -> Result<SignalRow, AdapterError> {
        match kind {
            SignalKind::Test => finish_collection(
                Language::Rust,
                kind,
                context,
                test::collect_selected_with_lines(context, selection, on_line),
            ),
            _ => self.collect_streaming(kind, context, on_line),
        }
    }
    fn collect_streaming(
        &self,
        kind: SignalKind,
        context: &RunContext,
        on_line: &mut dyn FnMut(&str),
    ) -> Result<SignalRow, AdapterError> {
        match kind {
            SignalKind::Test => finish_collection(
                Language::Rust,
                kind,
                context,
                test::collect_with_lines(context, on_line),
            ),
            _ => self.collect(kind, context),
        }
    }

    fn collect(&self, kind: SignalKind, context: &RunContext) -> Result<SignalRow, AdapterError> {
        let result: CollectorResult = match kind {
            SignalKind::Test => test::collect(context),
            SignalKind::Coverage => coverage::collect(context),
            SignalKind::Size => size::collect(context).map_err(CollectorError::Adapter),
            SignalKind::Complexity => complexity::collect(context),
            SignalKind::Deps => deps::collect(context),
            SignalKind::Mutation => Err(CollectorError::Adapter(String::from(
                "mutation is not supported for Rust targets",
            ))),
        };
        finish_collection(Language::Rust, kind, context, result)
    }
}

#[cfg(test)]
mod tests {
    use super::{RustCollector, cargo_subcommand_executable};
    use ayni_core::{
        AyniPolicy, ExecutionResolution, Language, RunContext, Scope, SignalCollector, SignalKind,
        SignalResult,
    };
    use std::fs;
    use std::time::Duration;
    use tempfile::TempDir;

    fn fixture_execution(cwd: std::path::PathBuf) -> ExecutionResolution {
        let mut execution = ExecutionResolution::direct("test", cwd, "test", 100);
        execution.environment.insert(
            ayni_adapters_common::exec::DISCARD_LLVM_PROFILE_ENV.to_string(),
            String::new(),
        );
        execution
    }

    #[test]
    fn cargo_subcommand_overrides_declare_their_executables() {
        let policy: AyniPolicy = toml::from_str(
            r#"
[checks]
test = true
coverage = true
[languages]
enabled = ["rust"]
[rust.tooling.test]
command = "cargo"
args = ["nextest", "run"]
[rust.tooling.coverage]
command = "/usr/local/bin/cargo"
"#,
        )
        .expect("policy");
        let cwd = std::env::current_dir().expect("working directory");
        let context = RunContext {
            repo_root: cwd.clone(),
            target_root: cwd.clone(),
            workdir: cwd.clone(),
            policy,
            scope: Scope::default(),
            execution: ExecutionResolution::direct("cargo", cwd, "test", 100),
            cancellation: Default::default(),
            debug: false,
        };

        assert_eq!(
            RustCollector.required_host_executables(SignalKind::Test, &context),
            vec!["cargo", "cargo-nextest"]
        );
        assert_eq!(
            RustCollector.required_host_executables(SignalKind::Coverage, &context),
            vec!["/usr/local/bin/cargo", "cargo-llvm-cov"]
        );
    }

    #[test]
    fn cargo_preflight_uses_only_the_actual_subcommand() {
        assert_eq!(
            cargo_subcommand_executable(
                SignalKind::Test,
                "cargo",
                &[
                    String::from("--color"),
                    String::from("always"),
                    String::from("-C"),
                    String::from("workspace"),
                    String::from("nextest")
                ],
            ),
            Some(String::from("cargo-nextest"))
        );
        assert_eq!(
            cargo_subcommand_executable(
                SignalKind::Test,
                "cargo",
                &[String::from("test"), String::from("nextest")],
            ),
            None
        );
        assert_eq!(
            cargo_subcommand_executable(
                SignalKind::Test,
                "cargo",
                &[
                    String::from("test"),
                    String::from("--"),
                    String::from("nextest"),
                ],
            ),
            None
        );
    }

    #[test]
    fn controlled_timeout_child() {
        std::thread::sleep(Duration::from_secs(2));
    }

    #[test]
    fn mutation_is_rejected_without_invoking_a_tool() {
        let policy: AyniPolicy = toml::from_str(
            r#"
[checks]
mutation = true

[languages]
enabled = ["rust"]
"#,
        )
        .expect("policy");
        let cwd = std::env::current_dir().expect("working directory");
        let context = RunContext {
            repo_root: cwd.clone(),
            target_root: cwd.clone(),
            workdir: cwd.clone(),
            policy,
            scope: Scope::default(),
            execution: ExecutionResolution::direct("cargo", cwd, "test", 100),
            cancellation: Default::default(),
            debug: false,
        };

        let error = RustCollector
            .collect(SignalKind::Mutation, &context)
            .expect_err("Rust mutation must be rejected");
        assert_eq!(error.language, Language::Rust);
        assert_eq!(error.message, "mutation is not supported for Rust targets");
    }

    #[test]
    fn coverage_backed_test_collection_requires_opt_in() {
        let policy: AyniPolicy =
            toml::from_str("[checks]\ntest=true\ncoverage=true\n[languages]\nenabled=[\"rust\"]")
                .expect("policy");
        let cwd = std::env::current_dir().expect("working directory");
        let context = RunContext {
            repo_root: cwd.clone(),
            target_root: cwd.clone(),
            workdir: cwd.clone(),
            policy,
            scope: Scope::default(),
            execution: ExecutionResolution::direct("cargo", cwd, "test", 100),
            cancellation: Default::default(),
            debug: false,
        };
        assert!(!RustCollector.supports_coverage_backed_test(&context));
    }

    #[cfg(unix)]
    #[test]
    fn opted_in_coverage_command_runs_once_and_emits_both_rows() {
        let directory = TempDir::new().expect("fixture");
        fs::write(
            directory.path().join("combined.sh"),
            r#"#!/bin/sh
printf 'launched\n' >> launches
printf '%s\n' '{"data":[{"totals":{"lines":{"percent":82.5},"branches":{"percent":71.0}}}]}'
printf '%s\n' 'test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out' >&2
"#,
        )
        .expect("script");
        let policy: AyniPolicy = toml::from_str(
            r#"
[checks]
test = true
coverage = true
[languages]
enabled = ["rust"]
[rust.tooling]
coverage_satisfies_test = true
[rust.tooling.coverage]
command = "sh"
args = ["combined.sh"]
"#,
        )
        .expect("policy");
        let root = directory.path().to_path_buf();
        let context = RunContext {
            repo_root: root.clone(),
            target_root: root.clone(),
            workdir: root.clone(),
            policy,
            scope: Scope::default(),
            execution: ExecutionResolution::direct("cargo", root, "test", 100),
            cancellation: Default::default(),
            debug: false,
        };

        let (test, coverage) = RustCollector
            .collect_coverage_backed_test(Language::Rust, &context, &mut |_| {})
            .expect("combined rows");

        assert_eq!(
            fs::read_to_string(directory.path().join("launches")).unwrap(),
            "launched\n"
        );
        assert!(test.pass);
        assert!(coverage.pass);
        let SignalResult::Test(result) = test.result else {
            panic!("test row");
        };
        assert_eq!(
            (result.total_tests, result.passed, result.failed),
            (7, 7, 0)
        );
        let SignalResult::Coverage(result) = coverage.result else {
            panic!("coverage row");
        };
        assert_eq!(result.line_percent, Some(82.5));
    }

    #[test]
    fn configured_timeout_is_failed_row() {
        let executable = std::env::current_exe().expect("test executable");
        let program = executable.display().to_string();
        let args = [
            String::from("collectors::tests::controlled_timeout_child"),
            String::from("--exact"),
            String::from("--nocapture"),
        ];
        let policy: AyniPolicy = toml::from_str(&format!(
            r#"
[checks]
test = true

[languages]
enabled = ["rust"]

[execution]
tool_timeout_seconds = 1

[rust.tooling.test]
command = {program:?}
args = [{args}]
"#,
            args = args
                .iter()
                .map(|arg| format!("{arg:?}"))
                .collect::<Vec<_>>()
                .join(", "),
        ))
        .expect("policy");
        let cwd = std::env::current_dir().expect("working directory");
        let context = RunContext {
            repo_root: cwd.clone(),
            target_root: cwd.clone(),
            workdir: cwd.clone(),
            policy,
            scope: Scope::default(),
            execution: fixture_execution(cwd.clone()),
            cancellation: Default::default(),
            debug: false,
        };

        let row = RustCollector
            .collect(SignalKind::Test, &context)
            .expect("timeout must become a failed row");
        assert!(!row.pass);
        assert_eq!(row.kind, SignalKind::Test);
        assert_eq!(row.language, Language::Rust);
        let failure = row.result.command_failure().expect("timeout failure");
        assert_eq!(failure.classification, "timeout");
        assert_eq!(failure.category, "repo_code_issue");
        assert_eq!(failure.command, format!("{} {}", program, args.join(" ")));
        assert_eq!(failure.cwd, cwd.display().to_string());
    }
}
