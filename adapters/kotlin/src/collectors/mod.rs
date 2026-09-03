mod complexity;
mod coverage;
mod deps;
mod mutation;
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
pub struct KotlinCollector;

impl SignalCollector for KotlinCollector {
    fn required_host_executables(&self, kind: SignalKind, context: &RunContext) -> Vec<String> {
        if let Some(command) = context.policy.tool_override_for(Language::Kotlin, kind) {
            return vec![command.command.clone()];
        }
        match kind {
            SignalKind::Test
            | SignalKind::Coverage
            | SignalKind::Complexity
            | SignalKind::Deps
            | SignalKind::Mutation => vec![context.execution.runner.clone()],
            SignalKind::Size => Vec::new(),
        }
    }

    fn supports_coverage_backed_test(&self, context: &RunContext) -> bool {
        context.policy.kotlin.tooling.coverage_satisfies_test
    }

    fn collect_coverage_backed_test(
        &self,
        language: Language,
        context: &RunContext,
        on_line: &mut dyn FnMut(&str),
    ) -> Result<(SignalRow, SignalRow), AdapterError> {
        if language != Language::Kotlin || !self.supports_coverage_backed_test(context) {
            return Err(AdapterError::new(
                Language::Kotlin,
                "coverage-backed test collection is not enabled for this Kotlin target",
            ));
        }
        finish_coverage_backed_test(
            Language::Kotlin,
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
                Language::Kotlin,
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
            SignalKind::Mutation => mutation::collect(context),
        };
        finish_collection(Language::Kotlin, kind, context, result)
    }
}

#[cfg(test)]
mod tests {
    use super::KotlinCollector;
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
    fn coverage_backed_test_collection_requires_opt_in() {
        let policy: AyniPolicy =
            toml::from_str("[checks]\ntest=true\ncoverage=true\n[languages]\nenabled=[\"kotlin\"]")
                .expect("policy");
        let cwd = std::env::current_dir().expect("working directory");
        let context = RunContext {
            repo_root: cwd.clone(),
            target_root: cwd.clone(),
            workdir: cwd.clone(),
            policy,
            scope: Scope::default(),
            execution: ExecutionResolution::direct("gradle", cwd, "test", 100),
            cancellation: Default::default(),
            debug: false,
        };
        assert!(!KotlinCollector.supports_coverage_backed_test(&context));
    }

    #[cfg(unix)]
    #[test]
    fn opted_in_coverage_command_runs_once_and_emits_both_rows() {
        let directory = TempDir::new().expect("fixture");
        fs::write(directory.path().join("combined.sh"), r#"#!/bin/sh
printf 'launched\n' >> launches
mkdir -p build/test-results/test build/reports/jacoco
printf '%s\n' '<testsuite tests="3" failures="0" errors="0" skipped="0"><testcase name="a"/><testcase name="b"/><testcase name="c"/></testsuite>' > build/test-results/test/results.xml
printf '%s\n' '<report><counter type="LINE" missed="2" covered="8"/><counter type="BRANCH" missed="1" covered="3"/></report>' > build/reports/jacoco/report.xml
"#).expect("script");
        let policy: AyniPolicy = toml::from_str(
            r#"
[checks]
test = true
coverage = true
[languages]
enabled = ["kotlin"]
[kotlin.tooling]
coverage_satisfies_test = true
[kotlin.tooling.coverage]
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
            execution: ExecutionResolution::direct("gradle", root, "test", 100),
            cancellation: Default::default(),
            debug: false,
        };
        let (test, coverage) = KotlinCollector
            .collect_coverage_backed_test(Language::Kotlin, &context, &mut |_| {})
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
            (3, 3, 0)
        );
        let SignalResult::Coverage(result) = coverage.result else {
            panic!("coverage row")
        };
        assert_eq!(result.line_percent, Some(80.0));
    }

    #[cfg(unix)]
    #[test]
    fn missing_coverage_evidence_fails_both_rows_with_test_counts() {
        let directory = TempDir::new().expect("fixture");
        fs::write(directory.path().join("combined.sh"), r#"#!/bin/sh
mkdir -p build/test-results/test
printf '%s\n' '<testsuite tests="2" failures="0" errors="0" skipped="0"><testcase name="a"/><testcase name="b"/></testsuite>' > build/test-results/test/results.xml
"#).expect("script");
        let policy: AyniPolicy = toml::from_str(
            r#"
[checks]
test = true
coverage = true
[languages]
enabled = ["kotlin"]
[kotlin.tooling]
coverage_satisfies_test = true
[kotlin.tooling.coverage]
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
            execution: ExecutionResolution::direct("gradle", root, "test", 100),
            cancellation: Default::default(),
            debug: false,
        };
        let (test, coverage) = KotlinCollector
            .collect_coverage_backed_test(Language::Kotlin, &context, &mut |_| {})
            .expect("typed failed rows");
        assert!(!test.pass);
        assert!(!coverage.pass);
        let SignalResult::Test(result) = test.result else {
            panic!("test row")
        };
        assert_eq!(
            (result.total_tests, result.passed, result.failed),
            (2, 2, 0)
        );
        assert_eq!(
            result.failure.unwrap().classification,
            "incomplete_combined_evidence"
        );
    }

    #[cfg(unix)]
    #[test]
    fn malformed_coverage_evidence_fails_both_rows_with_test_counts() {
        let directory = TempDir::new().expect("fixture");
        fs::write(directory.path().join("combined.sh"), r#"#!/bin/sh
mkdir -p build/test-results/test build/reports/kover
printf '%s\n' '<testsuite tests="2" failures="0" errors="0" skipped="0"><testcase name="a"/><testcase name="b"/></testsuite>' > build/test-results/test/results.xml
printf '%s\n' 'not XML coverage evidence' > build/reports/kover/report.xml
"#).expect("script");
        let policy: AyniPolicy = toml::from_str(
            r#"
[checks]
test = true
coverage = true
[languages]
enabled = ["kotlin"]
[kotlin.tooling]
coverage_satisfies_test = true
[kotlin.tooling.coverage]
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
            execution: ExecutionResolution::direct("gradle", root, "test", 100),
            cancellation: Default::default(),
            debug: false,
        };
        let (test, coverage) = KotlinCollector
            .collect_coverage_backed_test(Language::Kotlin, &context, &mut |_| {})
            .expect("typed failed rows");
        assert!(!test.pass);
        assert!(!coverage.pass);
        let SignalResult::Test(result) = test.result else {
            panic!("test row")
        };
        assert_eq!(
            (result.total_tests, result.passed, result.failed),
            (2, 2, 0)
        );
        assert_eq!(
            result.failure.unwrap().classification,
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
enabled = ["kotlin"]

[execution]
tool_timeout_seconds = 1

[kotlin.tooling.test]
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
            execution: ExecutionResolution::direct("gradle", cwd.clone(), "test", 100),
            cancellation: Default::default(),
            debug: false,
        };

        let row = KotlinCollector
            .collect(SignalKind::Test, &context)
            .expect("timeout must become a failed row");
        assert!(!row.pass);
        assert_eq!(row.kind, SignalKind::Test);
        assert_eq!(row.language, Language::Kotlin);
        let failure = row.result.command_failure().expect("timeout failure");
        assert_eq!(failure.classification, "timeout");
        assert_eq!(failure.category, "repo_code_issue");
        assert_eq!(failure.command, format!("{} {}", program, args.join(" ")));
        assert_eq!(failure.cwd, cwd.display().to_string());
    }
}
