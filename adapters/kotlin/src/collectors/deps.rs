use super::util::{command_failure_from_output, gradle_command, run_command_for_context};
use ayni_core::{
    Budget, DepsOffender, DepsResult, Language, Level, Offenders, RunContext, Scope, SignalKind,
    SignalResult, SignalRow,
};
use glob::Pattern;
use regex::Regex;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};

pub fn collect(context: &RunContext) -> Result<SignalRow, String> {
    let rules = context
        .policy
        .kotlin
        .deps
        .as_ref()
        .map(|value| value.forbidden.clone())
        .unwrap_or_default();
    let (program, args) = gradle_command(context, SignalKind::Deps, "dependencies");
    let output = run_command_for_context(context, &program, &args)?;
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
    let edges = parse_project_edges(&stdout, &from)?;
    let compiled_rules = compile_rules(&rules)?;
    let mut offenders = Vec::new();
    for (source, target) in &edges {
        for rule in &compiled_rules {
            if rule.from.matches(source) && rule.to.matches(target) {
                offenders.push(DepsOffender {
                    from: source.clone(),
                    to: target.clone(),
                    rule: format!("{} -> {}", rule.from_raw, rule.to_raw),
                    level: Level::Fail,
                });
            }
        }
    }
    offenders.sort_by(|left, right| {
        left.from
            .cmp(&right.from)
            .then_with(|| left.to.cmp(&right.to))
    });

    Ok(SignalRow {
        kind: SignalKind::Deps,
        language: Language::Kotlin,
        scope: Scope {
            workspace_root: context.scope.workspace_root.clone(),
            path: context.scope.path.clone(),
            package: context.scope.package.clone(),
            file: context.scope.file.clone(),
        },
        pass: offenders.is_empty(),
        result: SignalResult::Deps(DepsResult {
            crate_count: 1 + edges
                .iter()
                .map(|(_, to)| to)
                .collect::<BTreeSet<_>>()
                .len() as u64,
            edge_count: edges.len() as u64,
            violation_count: offenders.len() as u64,
        }),
        budget: Budget::Deps(json!({ "forbidden": rules })),
        offenders: Offenders::Deps(offenders),
        delta_vs_previous: None,
        delta_vs_baseline: None,
    })
}

fn error_row(context: &RunContext, _failure: ayni_core::CommandFailure) -> SignalRow {
    SignalRow {
        kind: SignalKind::Deps,
        language: Language::Kotlin,
        scope: Scope {
            workspace_root: context.scope.workspace_root.clone(),
            path: context.scope.path.clone(),
            package: context.scope.package.clone(),
            file: context.scope.file.clone(),
        },
        pass: false,
        result: SignalResult::Deps(DepsResult {
            crate_count: 0,
            edge_count: 0,
            violation_count: 1,
        }),
        budget: Budget::Deps(json!({})),
        offenders: Offenders::Deps(Vec::new()),
        delta_vs_previous: None,
        delta_vs_baseline: None,
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

struct CompiledRule {
    from_raw: String,
    to_raw: String,
    from: Pattern,
    to: Pattern,
}

fn compile_rules(map: &BTreeMap<String, Vec<String>>) -> Result<Vec<CompiledRule>, String> {
    let mut out = Vec::new();
    for (from, targets) in map {
        let from_pattern = Pattern::new(from)
            .map_err(|error| format!("invalid deps forbidden source glob '{from}': {error}"))?;
        for target in targets {
            out.push(CompiledRule {
                from_raw: from.clone(),
                to_raw: target.clone(),
                from: from_pattern.clone(),
                to: Pattern::new(target).map_err(|error| {
                    format!("invalid deps forbidden target glob '{target}': {error}")
                })?,
            });
        }
    }
    Ok(out)
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
