use ayni_core::{
    Budget, CommandFailure, MutationResult, Offenders, RunContext, Scope, SignalKind, SignalResult,
    SignalRow,
};
use serde_json::json;
use std::process::Command;

pub fn collect(context: &RunContext) -> Result<SignalRow, String> {
    let enabled = context.policy.checks.mutation;
    if !enabled {
        return Ok(SignalRow {
            kind: SignalKind::Mutation,
            language: ayni_core::Language::Rust,
            scope: Scope {
                workspace_root: context.scope.workspace_root.clone(),
                path: context.scope.path.clone(),
                package: context.scope.package.clone(),
                file: context.scope.file.clone(),
            },
            pass: true,
            result: SignalResult::Mutation(MutationResult {
                engine: String::from("cargo-mutants"),
                killed: 0,
                survived: 0,
                timeout: 0,
                score: None,
                failure: None,
            }),
            budget: Budget::Mutation(json!({"enabled": false})),
            offenders: Offenders::Mutation(Vec::new()),
            delta_vs_previous: None,
            delta_vs_baseline: None,
        });
    }

    let (program, args, engine_label) = mutation_command(context);
    let command_text = format_command(&program, &args);
    let output = Command::new(&program)
        .args(args.iter().map(String::as_str))
        .current_dir(&context.execution.exec_cwd)
        .output()
        .map_err(|error| format!("failed to execute {command_text}: {error}"))?;

    let status_ok = output.status.success();
    Ok(SignalRow {
        kind: SignalKind::Mutation,
        language: ayni_core::Language::Rust,
        scope: Scope {
            workspace_root: context.scope.workspace_root.clone(),
            path: context.scope.path.clone(),
            package: context.scope.package.clone(),
            file: context.scope.file.clone(),
        },
        pass: status_ok,
        result: SignalResult::Mutation(MutationResult {
            engine: engine_label,
            killed: 0,
            survived: if status_ok { 0 } else { 1 },
            timeout: 0,
            score: if status_ok { Some(1.0) } else { Some(0.0) },
            failure: (!status_ok).then(|| command_failure(context, &program, &args, &output)),
        }),
        budget: Budget::Mutation(json!({"enabled": true})),
        offenders: Offenders::Mutation(Vec::new()),
        delta_vs_previous: None,
        delta_vs_baseline: None,
    })
}

fn command_failure(
    context: &RunContext,
    program: &str,
    args: &[String],
    output: &std::process::Output,
) -> CommandFailure {
    CommandFailure {
        category: String::from("repo_code_issue"),
        classification: String::from("command_error"),
        command: format_command(program, args),
        cwd: context.execution.exec_cwd.display().to_string(),
        exit_code: output.status.code(),
        message: concise_failure_message(output),
    }
}

fn concise_failure_message(output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    format!("{stderr}\n{stdout}")
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| String::from("command failed without stdout/stderr output"))
}

fn mutation_command(context: &RunContext) -> (String, Vec<String>, String) {
    if let Some(override_cmd) = context
        .policy
        .tool_override_for(ayni_core::Language::Rust, SignalKind::Mutation)
    {
        let args = if override_cmd.args.is_empty() {
            vec![String::from("mutants"), String::from("--list")]
        } else {
            override_cmd.args.clone()
        };
        let engine = format_command(&override_cmd.command, &args);
        return (override_cmd.command.clone(), args, engine);
    }
    (
        String::from("cargo"),
        vec![String::from("mutants"), String::from("--list")],
        String::from("cargo-mutants"),
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
            diff: None,
            execution: ExecutionResolution::direct("cargo", PathBuf::from("."), "test", 100),
            debug: false,
        }
    }

    #[test]
    fn default_mutation_command_is_cargo_mutants() {
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
enabled = ["rust"]
"#,
        );
        let (program, args, engine) = mutation_command(&context);
        assert_eq!(program, "cargo");
        assert_eq!(args, vec!["mutants", "--list"]);
        assert_eq!(engine, "cargo-mutants");
    }

    #[test]
    fn mutation_command_uses_rust_tooling_override() {
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
enabled = ["rust"]

[rust.tooling.mutation]
command = "cargo"
args = ["mutants", "--in-diff", ".ayni/branch.diff"]
"#,
        );
        let (program, args, engine) = mutation_command(&context);
        assert_eq!(program, "cargo");
        assert_eq!(args, vec!["mutants", "--in-diff", ".ayni/branch.diff"]);
        assert_eq!(engine, "cargo mutants --in-diff .ayni/branch.diff");
    }
}
