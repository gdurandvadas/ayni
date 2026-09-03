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
pub struct NodeCollector;

impl SignalCollector for NodeCollector {
    fn required_host_executables(&self, kind: SignalKind, context: &RunContext) -> Vec<String> {
        if let Some(command) = context.policy.tool_override_for(Language::Node, kind) {
            return vec![command.command.clone()];
        }
        match kind {
            SignalKind::Test | SignalKind::Coverage | SignalKind::Complexity => {
                vec![context.execution.runner.clone()]
            }
            SignalKind::Size | SignalKind::Deps | SignalKind::Mutation => Vec::new(),
        }
    }

    fn supports_coverage_backed_test(&self, context: &RunContext) -> bool {
        let tooling = &context.policy.node.tooling;
        tooling.coverage_satisfies_test
    }

    fn collect_coverage_backed_test(
        &self,
        language: Language,
        context: &RunContext,
        on_line: &mut dyn FnMut(&str),
    ) -> Result<(SignalRow, SignalRow), AdapterError> {
        if language != Language::Node || !self.supports_coverage_backed_test(context) {
            return Err(AdapterError::new(
                Language::Node,
                "coverage-backed test collection is not enabled for this Node target",
            ));
        }
        finish_coverage_backed_test(
            Language::Node,
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
                Language::Node,
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
            SignalKind::Mutation => Err(CollectorError::Adapter(String::from(
                "mutation is not supported for Node targets",
            ))),
        };
        finish_collection(Language::Node, kind, context, result)
    }
}

#[cfg(test)]
mod tests {
    use super::NodeCollector;
    use ayni_core::{
        AyniPolicy, ExecutionResolution, Language, RunContext, Scope, SignalCollector, SignalKind,
        SignalResult,
    };
    use std::fs;
    use std::time::Duration;
    use tempfile::TempDir;

    #[test]
    fn controlled_timeout_child() {
        std::thread::sleep(Duration::from_secs(2));
    }

    #[test]
    fn mutation_is_rejected_without_invoking_an_override() {
        let policy: AyniPolicy = toml::from_str(
            r#"
[checks]
mutation = true

[languages]
enabled = ["node"]

[node.tooling.mutation]
command = "this-command-must-not-run"
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
            execution: ExecutionResolution::direct("node", cwd, "test", 100),
            cancellation: Default::default(),
            debug: false,
        };

        let error = NodeCollector
            .collect(SignalKind::Mutation, &context)
            .expect_err("Node mutation must be rejected");
        assert_eq!(error.language, Language::Node);
        assert_eq!(error.message, "mutation is not supported for Node targets");
    }

    #[test]
    fn coverage_backed_test_collection_requires_opt_in() {
        let policy: AyniPolicy =
            toml::from_str("[checks]\ntest=true\ncoverage=true\n[languages]\nenabled=[\"node\"]")
                .expect("policy");
        let cwd = std::env::current_dir().expect("working directory");
        let context = RunContext {
            repo_root: cwd.clone(),
            target_root: cwd.clone(),
            workdir: cwd.clone(),
            policy,
            scope: Scope::default(),
            execution: ExecutionResolution::direct("npm", cwd, "test", 100),
            cancellation: Default::default(),
            debug: false,
        };
        assert!(!NodeCollector.supports_coverage_backed_test(&context));
    }

    #[cfg(unix)]
    #[test]
    fn opted_in_coverage_command_runs_once_and_emits_both_rows() {
        let directory = TempDir::new().expect("fixture");
        fs::write(
            directory.path().join("combined.sh"),
            r#"#!/bin/sh
printf 'launched\n' >> launches
mkdir -p coverage
printf '%s\n' '{"total":{"lines":{"pct":84.0},"branches":{"pct":72.0}}}' > coverage/coverage-summary.json
printf '%s\n' '{"numTotalTests":9,"numPassedTests":9,"numFailedTests":0,"testResults":[]}'
"#,
        )
        .expect("script");
        let policy: AyniPolicy = toml::from_str(
            r#"
[checks]
test = true
coverage = true
[languages]
enabled = ["node"]
[node.tooling]
coverage_satisfies_test = true
[node.tooling.coverage]
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
            execution: ExecutionResolution::direct("npm", root, "test", 100),
            cancellation: Default::default(),
            debug: false,
        };

        let (test, coverage) = NodeCollector
            .collect_coverage_backed_test(Language::Node, &context, &mut |_| {})
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
            (9, 9, 0)
        );
        let SignalResult::Coverage(result) = coverage.result else {
            panic!("coverage row");
        };
        assert_eq!(result.line_percent, Some(84.0));
    }

    #[cfg(unix)]
    #[test]
    fn missing_coverage_report_rejects_otherwise_passing_test_evidence() {
        let directory = TempDir::new().expect("fixture");
        fs::write(
            directory.path().join("combined.sh"),
            "#!/bin/sh\nprintf '%s\\n' '{\"numTotalTests\":2,\"numPassedTests\":2,\"numFailedTests\":0,\"testResults\":[]}'\n",
        )
        .expect("script");
        let policy: AyniPolicy = toml::from_str(
            "[checks]\ntest=true\ncoverage=true\n[languages]\nenabled=[\"node\"]\n[node.tooling]\ncoverage_satisfies_test=true\n[node.tooling.coverage]\ncommand=\"sh\"\nargs=[\"combined.sh\"]",
        )
        .expect("policy");
        let root = directory.path().to_path_buf();
        let context = RunContext {
            repo_root: root.clone(),
            target_root: root.clone(),
            workdir: root.clone(),
            policy,
            scope: Scope::default(),
            execution: ExecutionResolution::direct("npm", root, "test", 100),
            cancellation: Default::default(),
            debug: false,
        };

        let (test, coverage) = NodeCollector
            .collect_coverage_backed_test(Language::Node, &context, &mut |_| {})
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
        let directory = TempDir::new().expect("fixture");
        fs::write(
            directory.path().join("combined.sh"),
            r#"#!/bin/sh
mkdir -p coverage
printf '%s\n' '{"total":{"lines":{"pct":40.0},"branches":{"pct":20.0}}}' > coverage/coverage-summary.json
printf '%s\n' '{"numTotalTests":3,"numPassedTests":2,"numFailedTests":1,"testResults":[{"name":"src/math.test.ts","status":"failed","assertionResults":[{"status":"failed","fullName":"adds values","failureMessages":["expected 3"]}]}]}'
exit 1
"#,
        )
        .expect("script");
        let policy: AyniPolicy = toml::from_str(
            "[checks]\ntest=true\ncoverage=true\n[languages]\nenabled=[\"node\"]\n[node.tooling]\ncoverage_satisfies_test=true\n[node.tooling.coverage]\ncommand=\"sh\"\nargs=[\"combined.sh\"]",
        )
        .expect("policy");
        let root = directory.path().to_path_buf();
        let context = RunContext {
            repo_root: root.clone(),
            target_root: root.clone(),
            workdir: root.clone(),
            policy,
            scope: Scope::default(),
            execution: ExecutionResolution::direct("npm", root, "test", 100),
            cancellation: Default::default(),
            debug: false,
        };

        let (test, coverage) = NodeCollector
            .collect_coverage_backed_test(Language::Node, &context, &mut |_| {})
            .expect("typed failed rows");
        assert!(!test.pass);
        assert!(!coverage.pass);
        let SignalResult::Test(result) = &test.result else {
            panic!("test row");
        };
        assert_eq!(
            (result.total_tests, result.passed, result.failed),
            (3, 2, 1)
        );
        let ayni_core::Offenders::Test(offenders) = &test.offenders else {
            panic!("test offenders");
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
enabled = ["node"]

[execution]
tool_timeout_seconds = 1

[node.tooling.test]
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

        let row = NodeCollector
            .collect(SignalKind::Test, &context)
            .expect("timeout must become a failed row");
        assert!(!row.pass);
        assert_eq!(row.kind, SignalKind::Test);
        assert_eq!(row.language, Language::Node);
        let failure = row.result.command_failure().expect("timeout failure");
        assert_eq!(failure.classification, "timeout");
        assert_eq!(failure.category, "repo_code_issue");
        assert_eq!(failure.command, format!("{} {}", program, args.join(" ")));
        assert_eq!(failure.cwd, cwd.display().to_string());
    }
}
