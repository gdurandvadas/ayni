use super::util::{command_failure_from_output, run_tool, tool_command};
use ayni_adapters_common::collector::CollectorResult;
use ayni_adapters_common::exec::{format_command, run_command_for_context_structured};
use ayni_core::{
    Budget, MutationResult, Offenders, RunContext, Scope, SignalKind, SignalResult, SignalRow,
};
use serde_json::json;

pub fn collect(context: &RunContext) -> CollectorResult {
    let enabled = context.policy.checks.mutation;
    if !enabled {
        return Ok(SignalRow {
            kind: SignalKind::Mutation,
            language: ayni_core::Language::Node,
            scope: Scope {
                workspace_root: context.scope.workspace_root.clone(),
                path: context.scope.path.clone(),
                package: context.scope.package.clone(),
                file: context.scope.file.clone(),
            },
            pass: true,
            result: SignalResult::Mutation(MutationResult {
                engine: String::from("stryker (experimental)"),
                killed: 0,
                survived: 0,
                timeout: 0,
                score: None,
                failure: None,
            }),
            budget: Budget::Mutation(json!({"enabled": false})),
            offenders: Offenders::Mutation(Vec::new()),
        });
    }

    let (output, engine) = if let Some((program, args, engine)) = mutation_override_command(context)
    {
        (
            run_command_for_context_structured(context, &program, &args)?,
            engine,
        )
    } else {
        let (program, args) = tool_command(context, "stryker", &["run", "--logLevel", "error"]);
        let engine = format_command(&program, &args);
        (
            run_tool(context, "stryker", &["run", "--logLevel", "error"])?,
            engine,
        )
    };
    let status_ok = output.status.success();
    let failure = if status_ok {
        Some(ayni_adapters_common::failure::setup_failure(
            context,
            engine.clone(),
            "Stryker completed, but Node mutation report normalization is still experimental and cannot produce trustworthy counts",
        ))
    } else {
        Some(command_failure_from_output(
            context,
            SignalKind::Mutation,
            engine.split_whitespace().next().unwrap_or("node"),
            &engine
                .split_whitespace()
                .skip(1)
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            &output,
        ))
    };
    Ok(SignalRow {
        kind: SignalKind::Mutation,
        language: ayni_core::Language::Node,
        scope: Scope {
            workspace_root: context.scope.workspace_root.clone(),
            path: context.scope.path.clone(),
            package: context.scope.package.clone(),
            file: context.scope.file.clone(),
        },
        // Until Stryker's report is normalized into typed counts and findings,
        // command success is not sufficient mutation evidence.
        pass: false,
        result: SignalResult::Mutation(MutationResult {
            engine,
            killed: 0,
            survived: 0,
            timeout: 0,
            // Experimental support relies on Stryker's exit status. Without
            // parsing its report, do not fabricate a mutation score.
            score: None,
            failure,
        }),
        budget: Budget::Mutation(json!({"enabled": true})),
        offenders: Offenders::Mutation(Vec::new()),
    })
}

fn mutation_override_command(context: &RunContext) -> Option<(String, Vec<String>, String)> {
    let override_cmd = context
        .policy
        .tool_override_for(ayni_core::Language::Node, SignalKind::Mutation)?;
    let args = if override_cmd.args.is_empty() {
        vec![
            String::from("run"),
            String::from("--logLevel"),
            String::from("error"),
        ]
    } else {
        override_cmd.args.clone()
    };
    let engine = format_command(&override_cmd.command, &args);
    Some((override_cmd.command.clone(), args, engine))
}

#[cfg(test)]
mod tests {
    use super::{collect, mutation_override_command};
    use ayni_core::{AyniPolicy, ExecutionResolution, RunContext, Scope};
    use std::path::PathBuf;

    fn context_with_policy(document: &str) -> RunContext {
        let policy: AyniPolicy = toml::from_str(document).expect("policy");
        RunContext {
            repo_root: PathBuf::from("."),
            target_root: PathBuf::from("."),
            workdir: PathBuf::from("."),
            policy,
            scope: Scope::default(),
            execution: ExecutionResolution::direct("npm", PathBuf::from("."), "test", 100),
            debug: false,
        }
    }

    #[test]
    fn experimental_mutation_fails_closed_without_fabricating_a_score() {
        let context = context_with_policy(
            r#"
[checks]
mutation = true

[languages]
enabled = ["node"]

[node.tooling.mutation]
command = "true"
args = []
"#,
        );

        let row = collect(&context).expect("experimental mutation row");
        assert!(!row.pass);
        let ayni_core::SignalResult::Mutation(result) = row.result else {
            panic!("expected mutation result");
        };
        assert_eq!(result.score, None);
        assert_eq!(
            result
                .failure
                .expect("fail-closed diagnostic")
                .classification,
            "missing_report"
        );
    }

    #[test]
    fn no_override_returns_none() {
        let context = context_with_policy(
            r#"
[checks]
test = false
coverage = false
size = false
complexity = false
deps = false
mutation = true

[languages]
enabled = ["node"]
"#,
        );
        assert!(mutation_override_command(&context).is_none());
    }

    #[test]
    fn mutation_override_command_uses_node_tooling_override() {
        let context = context_with_policy(
            r#"
[checks]
test = false
coverage = false
size = false
complexity = false
deps = false
mutation = true

[languages]
enabled = ["node"]

[node.tooling.mutation]
command = "pnpm"
args = ["exec", "stryker", "run"]
"#,
        );
        let (program, args, engine) =
            mutation_override_command(&context).expect("expected node mutation override");
        assert_eq!(program, "pnpm");
        assert_eq!(args, vec!["exec", "stryker", "run"]);
        assert_eq!(engine, "pnpm exec stryker run");
    }
}
