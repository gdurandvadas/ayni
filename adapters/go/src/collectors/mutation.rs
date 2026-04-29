use super::util::run_tool_owned;
use ayni_core::{
    Budget, Language, MutationResult, Offenders, RunContext, Scope, SignalKind, SignalResult,
    SignalRow,
};
use serde_json::json;

pub fn collect(context: &RunContext) -> Result<SignalRow, String> {
    let enabled = context.policy.checks.mutation;
    if !enabled {
        return Ok(SignalRow {
            kind: SignalKind::Mutation,
            language: Language::Go,
            scope: Scope {
                workspace_root: context.scope.workspace_root.clone(),
                path: context.scope.path.clone(),
                package: context.scope.package.clone(),
                file: context.scope.file.clone(),
            },
            pass: true,
            result: SignalResult::Mutation(MutationResult {
                engine: String::from("go test (mutation proxy)"),
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

    let (program, args, engine) = mutation_command(context);
    let output = run_tool_owned(&context.workdir, &program, &args)?;
    let status_ok = output.status.success();
    Ok(SignalRow {
        kind: SignalKind::Mutation,
        language: Language::Go,
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

fn mutation_command(context: &RunContext) -> (String, Vec<String>, String) {
    if let Some(override_cmd) = context
        .policy
        .tool_override_for(Language::Go, SignalKind::Mutation)
    {
        let args = if override_cmd.args.is_empty() {
            vec![String::from("test"), String::from("./...")]
        } else {
            override_cmd.args.clone()
        };
        let engine = format!(
            "{} (mutation proxy)",
            format_command(&override_cmd.command, &args)
        );
        return (override_cmd.command.clone(), args, engine);
    }
    let args = vec![String::from("test"), String::from("./...")];
    (
        String::from("go"),
        args.clone(),
        format!("{} (mutation proxy)", format_command("go", &args)),
    )
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
    use super::mutation_command;
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
    fn default_mutation_command_is_go_test_proxy() {
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
enabled = ["go"]
"#,
        );
        let (program, args, engine) = mutation_command(&context);
        assert_eq!(program, "go");
        assert_eq!(args, vec!["test", "./..."]);
        assert!(engine.contains("mutation proxy"));
    }

    #[test]
    fn mutation_command_uses_go_tooling_override() {
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
enabled = ["go"]

[go.tooling.mutation]
command = "go"
args = ["test", "./...", "-run", "MutationSuite"]
"#,
        );
        let (program, args, engine) = mutation_command(&context);
        assert_eq!(program, "go");
        assert_eq!(args, vec!["test", "./...", "-run", "MutationSuite"]);
        assert!(engine.contains("mutation proxy"));
    }
}
