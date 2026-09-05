use super::util::{gradle_command, prepare_gradle_execution};
use ayni_adapters_common::collector::{CollectorError, CollectorResult};
use ayni_adapters_common::deps::{compile_rules, matching_offenders};
use ayni_adapters_common::exec::run_command_for_context_structured;
use ayni_adapters_common::failure::command_failure_from_output;
use ayni_core::{
    Budget, DepsBudget, DepsResult, Language, Offenders, RunContext, SignalKind, SignalResult,
    SignalRow,
};
use regex::Regex;
use std::collections::BTreeSet;

pub fn collect(context: &RunContext) -> CollectorResult {
    prepare_gradle_execution(context, SignalKind::Deps).map_err(CollectorError::Adapter)?;
    let rules = context
        .policy
        .kotlin
        .deps
        .as_ref()
        .map(|value| value.forbidden.clone())
        .unwrap_or_default();
    let (program, args) = gradle_command(context, SignalKind::Deps, "dependencies");
    let output = run_command_for_context_structured(context, &program, &args)?;
    if !output.status.success() {
        return Ok(error_row(
            context,
            command_failure_from_output(context, SignalKind::Deps, &program, &args, &output),
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let from = context
        .scope
        .path
        .clone()
        .unwrap_or_else(|| String::from("."));
    let edges = parse_project_edges(&stdout, &from).map_err(CollectorError::Adapter)?;
    let compiled_rules = compile_rules(&rules).map_err(CollectorError::Adapter)?;
    let offenders = matching_offenders(&edges, &compiled_rules);

    Ok(SignalRow {
        kind: SignalKind::Deps,
        language: Language::Kotlin,
        scope: context.scope.clone(),
        pass: offenders.is_empty(),
        result: SignalResult::Deps(DepsResult {
            crate_count: 1 + edges
                .iter()
                .map(|(_, to)| to)
                .collect::<BTreeSet<_>>()
                .len() as u64,
            edge_count: edges.len() as u64,
            violation_count: offenders.len() as u64,
            failure: None,
        }),
        budget: Budget::Deps(DepsBudget {
            forbidden: Some(rules),
        }),
        offenders: Offenders::Deps(offenders),
    })
}

fn error_row(context: &RunContext, failure: ayni_core::CommandFailure) -> SignalRow {
    SignalRow {
        kind: SignalKind::Deps,
        language: Language::Kotlin,
        scope: context.scope.clone(),
        pass: false,
        result: SignalResult::Deps(DepsResult {
            crate_count: 0,
            edge_count: 0,
            violation_count: 1,
            failure: Some(failure),
        }),
        budget: Budget::Deps(DepsBudget::default()),
        offenders: Offenders::Deps(Vec::new()),
    }
}

fn parse_project_edges(output: &str, from: &str) -> Result<BTreeSet<(String, String)>, String> {
    let re = Regex::new(r#"project\s+(:[A-Za-z0-9_.:-]+)"#)
        .map_err(|error| format!("failed to compile project dependency regex: {error}"))?;
    let mut edges = BTreeSet::new();
    for caps in re.captures_iter(output) {
        let Some(target) = caps
            .get(1)
            .map(|value| gradle_path_to_rule_path(value.as_str()))
        else {
            continue;
        };
        if target != from {
            edges.insert((from.to_string(), target));
        }
    }
    Ok(edges)
}

fn gradle_path_to_rule_path(path: &str) -> String {
    let trimmed = path.trim_matches(':').replace(':', "/");
    if trimmed.is_empty() {
        String::from(".")
    } else {
        trimmed
    }
}

#[cfg(test)]
mod tests {
    use super::parse_project_edges;

    #[test]
    fn parses_gradle_project_edges() {
        let edges = parse_project_edges(
            r#"
compileClasspath
\--- project :libs:domain
runtimeClasspath
\--- project :apps:web
"#,
            "apps/api",
        )
        .expect("edges");

        assert!(edges.contains(&(String::from("apps/api"), String::from("libs/domain"))));
        assert!(edges.contains(&(String::from("apps/api"), String::from("apps/web"))));
    }
}
