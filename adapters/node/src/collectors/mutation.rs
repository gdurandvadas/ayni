use super::util::package_manager_for_context;
use super::util::{run_command, run_tool};
use ayni_core::{
    Budget, MutationResult, Offenders, RunContext, Scope, SignalKind, SignalResult, SignalRow,
};
use serde_json::json;

pub fn collect(context: &RunContext) -> Result<SignalRow, String> {
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
                engine: String::from("stryker"),
                killed: 0,
                survived: 0,
                timeout: 0,
                score: None,
            }),
            budget: Budget::Mutation(json!({"enabled": false})),
            offenders: Offenders::Mutation(Vec::new()),
            delta_vs_previous: None,
            delta_vs_baseline: None,
        });
    }

    let (output, engine) = if let Some((program, args, engine)) = mutation_override_command(context)
    {
        (run_command(&context.workdir, &program, &args)?, engine)
    } else {
        let output = run_tool(context, "stryker", &["run", "--logLevel", "error"])?;
        let manager = package_manager_for_context(context);
        (output, format!("{} exec stryker", manager.executable()))
    };
    let status_ok = output.status.success();
    Ok(SignalRow {
        kind: SignalKind::Mutation,
        language: ayni_core::Language::Node,
        scope: Scope {
            workspace_root: context.scope.workspace_root.clone(),
            path: context.scope.path.clone(),
            package: context.scope.package.clone(),
            file: context.scope.file.clone(),
        },
        pass: status_ok,
        result: SignalResult::Mutation(MutationResult {
            engine,
            killed: 0,
            survived: if status_ok { 0 } else { 1 },
            timeout: 0,
            score: if status_ok { Some(1.0) } else { Some(0.0) },
        }),
        budget: Budget::Mutation(json!({"enabled": true})),
        offenders: Offenders::Mutation(Vec::new()),
        delta_vs_previous: None,
        delta_vs_baseline: None,
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

fn format_command(program: &str, args: &[String]) -> String {
    if args.is_empty() {
        program.to_string()
    } else {
        format!("{program} {}", args.join(" "))
    }
}

#[cfg(test)]
mod tests {
    use super::mutation_override_command;
    use ayni_core::{AyniPolicy, RunContext, Scope};
    use std::path::PathBuf;

    fn context_with_policy(document: &str) -> RunContext {
        let policy: AyniPolicy = toml::from_str(document).expect("policy");
        RunContext {
            repo_root: PathBuf::from("."),
            workdir: PathBuf::from("."),
            policy,
            scope: Scope::default(),
            diff: None,
        }
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
