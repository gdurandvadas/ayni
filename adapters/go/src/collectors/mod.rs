mod complexity;
mod coverage;
mod deps;
mod size;
mod test;
mod util;

use ayni_adapters_common::collector::{CollectorError, CollectorResult, finish_collection};
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
    };
    use std::time::Duration;

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
            execution: ExecutionResolution::direct("test", cwd.clone(), "test", 100),
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
