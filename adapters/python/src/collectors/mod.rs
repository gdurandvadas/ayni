use ayni_adapters_common::collector::{CollectorError, CollectorResult, finish_collection};
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
    };
    use std::time::Duration;

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
