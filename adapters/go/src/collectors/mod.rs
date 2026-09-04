mod complexity;
mod coverage;
mod deps;
mod size;
mod test;
mod util;

use ayni_adapters_common::collector::{
    CollectorError, CollectorResult, finish_collection, finish_coverage_backed_test,
};
use ayni_core::{
    AdapterError, Language, RunContext, SignalCollector, SignalKind, SignalRow,
    VerificationSelection,
};

#[derive(Debug, Default)]
pub struct GoCollector;

impl SignalCollector for GoCollector {
    fn required_host_executables(&self, kind: SignalKind, context: &RunContext) -> Vec<String> {
        let override_command = context
            .policy
            .tool_override_for(Language::Go, kind)
            .map(|command| command.command.clone());
        match kind {
            SignalKind::Test => {
                override_command.map_or_else(|| vec![String::from("go")], |command| vec![command])
            }
            SignalKind::Coverage => {
                let mut commands = vec![override_command.unwrap_or_else(|| String::from("go"))];
                if !commands.iter().any(|command| command == "go") {
                    commands.push(String::from("go"));
                }
                commands
            }
            SignalKind::Complexity => vec![String::from("gocyclo")],
            SignalKind::Deps => vec![String::from("go")],
            SignalKind::Size | SignalKind::Mutation => Vec::new(),
        }
    }

    fn supports_coverage_backed_test(&self, context: &RunContext) -> bool {
        context.policy.go.tooling.coverage_satisfies_test
    }

    fn collect_coverage_backed_test(
        &self,
        language: Language,
        context: &RunContext,
        on_line: &mut dyn FnMut(&str),
    ) -> Result<(SignalRow, SignalRow), AdapterError> {
        if language != Language::Go || !self.supports_coverage_backed_test(context) {
            return Err(AdapterError::new(
                Language::Go,
                "coverage-backed test collection is not enabled for this Go target",
            ));
        }
        finish_coverage_backed_test(
            Language::Go,
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
                Language::Go,
                kind,
                context,
                test::collect_selected(context, selection, on_line),
            ),
            _ => self.collect_streaming(kind, context, on_line),
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
                "mutation is not supported for Go targets",
            ))),
        };
        finish_collection(Language::Go, kind, context, result)
    }
}

#[cfg(test)]
mod tests {
    use super::GoCollector;
    use ayni_core::{
        AyniPolicy, ExecutionResolution, Language, RunContext, Scope, SignalCollector, SignalKind,
        SignalResult,
    };
    use std::{fs, path::Path, time::Duration};

    fn fixture_execution(cwd: std::path::PathBuf) -> ExecutionResolution {
        let mut execution = ExecutionResolution::direct("test", cwd, "test", 100);
        execution.environment.insert(
            ayni_adapters_common::exec::DISCARD_LLVM_PROFILE_ENV.to_string(),
            String::new(),
        );
        execution
    }

    #[test]
    fn host_preflight_keeps_go_cover_tool_after_a_coverage_override() {
        let policy: AyniPolicy = toml::from_str(
            r#"
[checks]
coverage = true

[languages]
enabled = ["go"]

[go.tooling.coverage]
command = "custom-go-coverage"
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
            execution: ExecutionResolution::direct("go", cwd, "test", 100),
            cancellation: Default::default(),
            debug: false,
        };

        assert_eq!(
            GoCollector.required_host_executables(SignalKind::Coverage, &context),
            ["custom-go-coverage", "go"]
        );
    }

    #[test]
    fn controlled_timeout_child() {
        std::thread::sleep(Duration::from_secs(2));
    }

    #[cfg(unix)]
    fn install_fake_go(root: &Path, context: &mut RunContext) {
        use std::os::unix::fs::PermissionsExt;

        let bin = root.join("fake-bin");
        fs::create_dir(&bin).expect("fake bin");
        let executable = bin.join("go");
        fs::write(
            &executable,
            "#!/bin/sh\nif [ \"$1\" = tool ] && [ \"$2\" = cover ] && [ -f \"$4\" ]; then\n  printf '%s\\n' 'total: (statements) 100.0%'\n  exit 0\nfi\nprintf '%s\\n' 'coverage profile missing' >&2\nexit 1\n",
        )
        .expect("fake go");
        let mut permissions = fs::metadata(&executable)
            .expect("fake go metadata")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&executable, permissions).expect("fake go executable");
        let inherited = std::env::var("PATH").unwrap_or_default();
        context.execution.environment.insert(
            String::from("PATH"),
            format!("{}:{inherited}", bin.display()),
        );
    }

    #[test]
    fn mutation_is_rejected_without_invoking_a_tool() {
        let policy: AyniPolicy = toml::from_str(
            r#"
[checks]
mutation = true

[languages]
enabled = ["go"]
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
            execution: ExecutionResolution::direct("go", cwd, "test", 100),
            cancellation: Default::default(),
            debug: false,
        };

        let error = GoCollector
            .collect(SignalKind::Mutation, &context)
            .expect_err("Go mutation must be rejected");
        assert_eq!(error.language, Language::Go);
        assert_eq!(error.message, "mutation is not supported for Go targets");
    }

    #[test]
    fn coverage_backed_test_collection_requires_opt_in() {
        let policy: AyniPolicy =
            toml::from_str("[checks]\ntest=true\ncoverage=true\n[languages]\nenabled=[\"go\"]")
                .expect("policy");
        let cwd = std::env::current_dir().expect("working directory");
        let context = RunContext {
            repo_root: cwd.clone(),
            target_root: cwd.clone(),
            workdir: cwd.clone(),
            policy,
            scope: Scope::default(),
            execution: ExecutionResolution::direct("go", cwd, "test", 100),
            cancellation: Default::default(),
            debug: false,
        };
        assert!(!GoCollector.supports_coverage_backed_test(&context));
    }

    #[cfg(unix)]
    #[test]
    fn opted_in_coverage_command_runs_once_and_emits_both_rows() {
        let directory = tempfile::TempDir::new().expect("fixture");
        fs::write(
            directory.path().join("sample.go"),
            "package sample\n\nfunc Value() int { return 1 }\n",
        )
        .expect("source");
        fs::write(
            directory.path().join("combined.sh"),
            r#"#!/bin/sh
printf 'launched\n' >> launches
for arg in "$@"; do
  case "$arg" in -coverprofile=*) profile=${arg#-coverprofile=} ;; esac
done
printf 'mode: set\n%s:3.1,3.24 1 1\n' "$PWD/sample.go" > "$profile"
printf '%s\n' '{"Action":"start","Package":"example.com/sample"}'
printf '%s\n' '{"Action":"run","Package":"example.com/sample","Test":"TestValue"}'
printf '%s\n' '{"Action":"pass","Package":"example.com/sample","Test":"TestValue","Elapsed":0.01}'
printf '%s\n' '{"Action":"pass","Package":"example.com/sample"}'
"#,
        )
        .expect("script");
        let policy: AyniPolicy = toml::from_str(
            r#"
[checks]
test = true
coverage = true
[languages]
enabled = ["go"]
[go.tooling]
coverage_satisfies_test = true
[go.tooling.coverage]
command = "sh"
args = ["combined.sh"]
"#,
        )
        .expect("policy");
        let root = directory.path().to_path_buf();
        let mut context = RunContext {
            repo_root: root.clone(),
            target_root: root.clone(),
            workdir: root.clone(),
            policy,
            scope: Scope::default(),
            execution: ExecutionResolution::direct("go", root, "test", 100),
            cancellation: Default::default(),
            debug: false,
        };
        install_fake_go(directory.path(), &mut context);

        let (test, coverage) = GoCollector
            .collect_coverage_backed_test(Language::Go, &context, &mut |_| {})
            .expect("combined rows");

        assert_eq!(
            fs::read_to_string(directory.path().join("launches")).unwrap(),
            "launched\n"
        );
        assert!(test.pass);
        assert!(coverage.pass);
        let SignalResult::Test(result) = test.result else {
            panic!("test row")
        };
        assert_eq!(
            (result.total_tests, result.passed, result.failed),
            (1, 1, 0)
        );
        let SignalResult::Coverage(result) = coverage.result else {
            panic!("coverage row")
        };
        assert_eq!(result.line_percent, Some(100.0));
    }

    #[cfg(unix)]
    #[test]
    fn missing_coverage_profile_rejects_otherwise_passing_test_evidence() {
        let directory = tempfile::TempDir::new().expect("fixture");
        fs::write(
            directory.path().join("combined.sh"),
            "#!/bin/sh\nprintf '%s\\n' '{\"Action\":\"start\",\"Package\":\"example.com/sample\"}'\nprintf '%s\\n' '{\"Action\":\"run\",\"Package\":\"example.com/sample\",\"Test\":\"TestValue\"}'\nprintf '%s\\n' '{\"Action\":\"pass\",\"Package\":\"example.com/sample\",\"Test\":\"TestValue\"}'\nprintf '%s\\n' '{\"Action\":\"pass\",\"Package\":\"example.com/sample\"}'\n",
        )
        .expect("script");
        let policy: AyniPolicy = toml::from_str(
            "[checks]\ntest=true\ncoverage=true\n[languages]\nenabled=[\"go\"]\n[go.tooling]\ncoverage_satisfies_test=true\n[go.tooling.coverage]\ncommand=\"sh\"\nargs=[\"combined.sh\"]",
        )
        .expect("policy");
        let root = directory.path().to_path_buf();
        let mut context = RunContext {
            repo_root: root.clone(),
            target_root: root.clone(),
            workdir: root.clone(),
            policy,
            scope: Scope::default(),
            execution: ExecutionResolution::direct("go", root, "test", 100),
            cancellation: Default::default(),
            debug: false,
        };
        install_fake_go(directory.path(), &mut context);

        let (test, coverage) = GoCollector
            .collect_coverage_backed_test(Language::Go, &context, &mut |_| {})
            .expect("typed failed rows");
        assert!(!test.pass);
        assert!(!coverage.pass);
        assert_eq!(
            test.result.command_failure().unwrap().classification,
            "incomplete_combined_evidence"
        );
    }

    #[cfg(unix)]
    #[test]
    fn failed_tests_fail_both_reused_rows_with_counts_and_findings() {
        let directory = tempfile::TempDir::new().expect("fixture");
        fs::write(
            directory.path().join("combined.sh"),
            r#"#!/bin/sh
printf '%s\n' '{"Action":"start","Package":"example.com/sample"}'
printf '%s\n' '{"Action":"run","Package":"example.com/sample","Test":"TestValue"}'
printf '%s\n' '{"Action":"fail","Package":"example.com/sample","Test":"TestValue","Elapsed":0.01}'
printf '%s\n' '{"Action":"fail","Package":"example.com/sample"}'
exit 1
"#,
        )
        .expect("script");
        let policy: AyniPolicy = toml::from_str(
            "[checks]\ntest=true\ncoverage=true\n[languages]\nenabled=[\"go\"]\n[go.tooling]\ncoverage_satisfies_test=true\n[go.tooling.coverage]\ncommand=\"sh\"\nargs=[\"combined.sh\"]",
        )
        .expect("policy");
        let root = directory.path().to_path_buf();
        let mut context = RunContext {
            repo_root: root.clone(),
            target_root: root.clone(),
            workdir: root.clone(),
            policy,
            scope: Scope::default(),
            execution: ExecutionResolution::direct("go", root, "test", 100),
            cancellation: Default::default(),
            debug: false,
        };
        install_fake_go(directory.path(), &mut context);

        let (test, coverage) = GoCollector
            .collect_coverage_backed_test(Language::Go, &context, &mut |_| {})
            .expect("typed failed rows");
        assert!(!test.pass);
        assert!(!coverage.pass);
        let SignalResult::Test(result) = &test.result else {
            panic!("test row")
        };
        assert_eq!(
            (result.total_tests, result.passed, result.failed),
            (1, 0, 1)
        );
        let ayni_core::Offenders::Test(offenders) = &test.offenders else {
            panic!("test offenders")
        };
        assert_eq!(offenders.len(), 1);
        assert!(coverage.result.command_failure().is_some());
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
enabled = ["go"]

[execution]
tool_timeout_seconds = 1

[go.tooling.test]
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

        let row = GoCollector
            .collect(SignalKind::Test, &context)
            .expect("timeout failed row");
        assert!(!row.pass);
        assert_eq!(row.kind, SignalKind::Test);
        assert_eq!(row.language, Language::Go);
        let failure = row.result.command_failure().expect("timeout failure");
        assert_eq!(failure.classification, "timeout");
        assert_eq!(failure.category, "repo_code_issue");
        assert_eq!(failure.command, format!("{} {}", program, args.join(" ")));
        assert_eq!(failure.cwd, cwd.display().to_string());
    }
}
