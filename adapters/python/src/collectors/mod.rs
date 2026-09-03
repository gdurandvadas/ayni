use ayni_adapters_common::collector::{
    CollectorError, CollectorResult, finish_collection, finish_coverage_backed_test,
};
use ayni_core::{
    AdapterError, Language, RunContext, SignalCollector, SignalKind, SignalRow,
    VerificationSelection,
};

pub mod complexity;
pub mod coverage;
pub mod deps;
pub mod mutation;
pub mod size;
pub mod test;
pub mod util;

#[derive(Debug, Default)]
pub struct PythonCollector;

impl SignalCollector for PythonCollector {
    fn required_host_executables(&self, kind: SignalKind, context: &RunContext) -> Vec<String> {
        if let Some(command) = context.policy.tool_override_for(Language::Python, kind) {
            return vec![command.command.clone()];
        }
        let manager = util::package_manager_for_context(context);
        match kind {
            SignalKind::Complexity if manager.executable() == "python" => {
                vec![String::from("complexipy")]
            }
            SignalKind::Test
            | SignalKind::Coverage
            | SignalKind::Complexity
            | SignalKind::Mutation => vec![manager.executable().to_string()],
            SignalKind::Size | SignalKind::Deps => Vec::new(),
        }
    }

    fn supports_coverage_backed_test(&self, context: &RunContext) -> bool {
        context.policy.python.tooling.coverage_satisfies_test
    }

    fn collect_coverage_backed_test(
        &self,
        language: Language,
        context: &RunContext,
        on_line: &mut dyn FnMut(&str),
    ) -> Result<(SignalRow, SignalRow), AdapterError> {
        if language != Language::Python || !self.supports_coverage_backed_test(context) {
            return Err(AdapterError::new(
                Language::Python,
                "coverage-backed test collection is not enabled for this Python target",
            ));
        }
        finish_coverage_backed_test(
            Language::Python,
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
                Language::Python,
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
            SignalKind::Deps => deps::collect(context).map_err(CollectorError::Adapter),
            SignalKind::Mutation => mutation::collect(context),
        };
        finish_collection(Language::Python, kind, context, result)
    }
}

#[cfg(test)]
mod tests {
    use super::PythonCollector;
    use ayni_core::{
        AyniPolicy, ExecutionResolution, Language, RunContext, Scope, SignalCollector, SignalKind,
        SignalResult,
    };
    use std::fs;
    use std::time::Duration;
    use tempfile::TempDir;

    #[test]
    fn host_preflight_matches_direct_and_managed_python_complexity_launchers() {
        let cwd = std::env::current_dir().expect("working directory");
        let mut context = RunContext {
            repo_root: cwd.clone(),
            target_root: cwd.clone(),
            workdir: cwd.clone(),
            policy: AyniPolicy::default(),
            scope: Scope::default(),
            execution: ExecutionResolution::direct("python3", cwd.clone(), "test", 100),
            cancellation: Default::default(),
            debug: false,
        };

        assert_eq!(
            PythonCollector.required_host_executables(SignalKind::Complexity, &context),
            ["complexipy"]
        );
        assert_eq!(
            PythonCollector.required_host_executables(SignalKind::Test, &context),
            ["python"]
        );
        context.execution.runner = String::from("uv");
        assert_eq!(
            PythonCollector.required_host_executables(SignalKind::Complexity, &context),
            ["uv"]
        );
    }

    #[test]
    fn controlled_timeout_child() {
        std::thread::sleep(Duration::from_secs(2));
    }

    #[test]
    fn coverage_backed_test_collection_requires_opt_in() {
        let policy: AyniPolicy =
            toml::from_str("[checks]\ntest=true\ncoverage=true\n[languages]\nenabled=[\"python\"]")
                .expect("policy");
        let cwd = std::env::current_dir().expect("working directory");
        let context = RunContext {
            repo_root: cwd.clone(),
            target_root: cwd.clone(),
            workdir: cwd.clone(),
            policy,
            scope: Scope::default(),
            execution: ExecutionResolution::direct("python", cwd, "test", 100),
            cancellation: Default::default(),
            debug: false,
        };
        assert!(!PythonCollector.supports_coverage_backed_test(&context));
    }

    #[cfg(unix)]
    #[test]
    fn opted_in_coverage_command_runs_once_and_emits_both_rows() {
        let directory = TempDir::new().expect("fixture");
        fs::write(
            directory.path().join("combined.sh"),
            r#"#!/bin/sh
printf 'launched\n' >> launches
mkdir -p .ayni/work/python/workspace
printf '%s\n' '{"duration":1.5,"summary":{"total":2,"passed":2,"failed":0,"error":0},"tests":[]}' > .ayni/work/python/workspace/pytest-report.json
printf '%s\n' '{"totals":{"covered_lines":8,"num_statements":10,"covered_branches":3,"num_branches":4}}' > .ayni/work/python/workspace/coverage.json
"#,
        )
        .expect("script");
        let policy: AyniPolicy = toml::from_str(
            r#"
[checks]
test = true
coverage = true
[languages]
enabled = ["python"]
[python.tooling]
coverage_satisfies_test = true
[python.tooling.coverage]
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
            execution: ExecutionResolution::direct("uv", root, "test", 100),
            cancellation: Default::default(),
            debug: false,
        };

        let (test, coverage) = PythonCollector
            .collect_coverage_backed_test(Language::Python, &context, &mut |_| {})
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
            (2, 2, 0)
        );
        let SignalResult::Coverage(result) = coverage.result else {
            panic!("coverage row")
        };
        assert_eq!(result.line_percent, Some(80.0));
        assert_eq!(result.branch_percent, Some(75.0));
    }

    #[cfg(unix)]
    #[test]
    fn incomplete_combined_evidence_fails_both_rows() {
        let directory = TempDir::new().expect("fixture");
        fs::write(
            directory.path().join("combined.sh"),
            "#!/bin/sh\nmkdir -p .ayni/work/python/workspace\nprintf '%s\n' '{\"duration\":1.5,\"summary\":{\"total\":2,\"passed\":2,\"failed\":0,\"error\":0},\"tests\":[]}' > .ayni/work/python/workspace/pytest-report.json\n",
        )
        .expect("script");
        let policy: AyniPolicy = toml::from_str(
            "[checks]\ntest=true\ncoverage=true\n[languages]\nenabled=[\"python\"]\n[python.tooling]\ncoverage_satisfies_test=true\n[python.tooling.coverage]\ncommand=\"sh\"\nargs=[\"combined.sh\"]",
        )
        .expect("policy");
        let root = directory.path().to_path_buf();
        let context = RunContext {
            repo_root: root.clone(),
            target_root: root.clone(),
            workdir: root.clone(),
            policy,
            scope: Scope::default(),
            execution: ExecutionResolution::direct("uv", root, "test", 100),
            cancellation: Default::default(),
            debug: false,
        };
        let (test, coverage) = PythonCollector
            .collect_coverage_backed_test(Language::Python, &context, &mut |_| {})
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
    fn malformed_test_evidence_fails_both_combined_rows() {
        let directory = TempDir::new().expect("fixture");
        fs::write(
            directory.path().join("combined.sh"),
            "#!/bin/sh\nmkdir -p .ayni/work/python/workspace\nprintf '{' > .ayni/work/python/workspace/pytest-report.json\nprintf '%s\n' '{\"totals\":{\"covered_lines\":8,\"num_statements\":10}}' > .ayni/work/python/workspace/coverage.json\n",
        )
        .expect("script");
        let policy: AyniPolicy = toml::from_str(
            "[checks]\ntest=true\ncoverage=true\n[languages]\nenabled=[\"python\"]\n[python.tooling]\ncoverage_satisfies_test=true\n[python.tooling.coverage]\ncommand=\"sh\"\nargs=[\"combined.sh\"]",
        )
        .expect("policy");
        let root = directory.path().to_path_buf();
        let context = RunContext {
            repo_root: root.clone(),
            target_root: root.clone(),
            workdir: root.clone(),
            policy,
            scope: Scope::default(),
            execution: ExecutionResolution::direct("uv", root, "test", 100),
            cancellation: Default::default(),
            debug: false,
        };
        let (test, coverage) = PythonCollector
            .collect_coverage_backed_test(Language::Python, &context, &mut |_| {})
            .expect("typed failed rows");
        assert!(!test.pass);
        assert!(!coverage.pass);
        assert_eq!(
            test.result.command_failure().unwrap().classification,
            "missing_report"
        );
        assert_eq!(
            coverage.result.command_failure().unwrap().classification,
            "incomplete_combined_evidence"
        );
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
enabled = ["python"]

[execution]
tool_timeout_seconds = 1

[python.tooling.test]
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
            execution: ExecutionResolution::direct("test", cwd.clone(), "test", 100),
            cancellation: Default::default(),
            debug: false,
        };

        let row = PythonCollector
            .collect(SignalKind::Test, &context)
            .expect("timeout must become a failed row");
        assert!(!row.pass);
        assert_eq!(row.kind, SignalKind::Test);
        assert_eq!(row.language, Language::Python);
        let failure = row.result.command_failure().expect("timeout failure");
        assert_eq!(failure.classification, "timeout");
        assert_eq!(failure.category, "repo_code_issue");
        assert_eq!(failure.command, format!("{} {}", program, args.join(" ")));
        assert_eq!(failure.cwd, cwd.display().to_string());
    }
}
