use ayni_adapters_common::collector::{CollectorError, CollectorResult};
use ayni_adapters_common::exec::run_command_for_context_structured;
use ayni_adapters_common::failure::command_failure_from_output;
use ayni_adapters_common::paths::{resolve_repo_path, to_repo_relative_path};
use ayni_core::{
    Budget, DepsBudget, DepsOffender, DepsResult, Language, Level, Offenders, RunContext, Scope,
    SignalKind, SignalResult, SignalRow,
};
use glob::Pattern;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

#[derive(Debug, Clone, Deserialize)]
struct GoPackage {
    #[serde(rename = "ImportPath")]
    import_path: String,
    #[serde(rename = "Dir")]
    dir: String,
    #[serde(rename = "Imports", default)]
    imports: Vec<String>,
}

#[derive(Debug, Clone)]
struct GoMember {
    import_path: String,
    dir: String,
}

pub fn collect(context: &RunContext) -> CollectorResult {
    let rules = context
        .policy
        .go
        .deps
        .as_ref()
        .map(|value| value.forbidden.clone())
        .unwrap_or_default();

    let args = vec![
        String::from("list"),
        String::from("-json"),
        String::from("./..."),
    ];
    let output = run_command_for_context_structured(context, "go", &args)?;
    if !output.status.success() {
        return Ok(error_row(
            context,
            command_failure_from_output(context, SignalKind::Deps, "go", &args, &output),
        ));
    }

    let reader = std::io::Cursor::new(output.stdout);
    let packages: Vec<GoPackage> = serde_json::Deserializer::from_reader(reader)
        .into_iter::<GoPackage>()
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            CollectorError::Adapter(format!("failed to parse go list output: {error}"))
        })?;

    let members = packages
        .iter()
        .map(|pkg| GoMember {
            import_path: pkg.import_path.clone(),
            dir: to_repo_relative_path(&context.repo_root, Path::new(&pkg.dir)),
        })
        .collect::<Vec<_>>();

    let visible = visible_members(&members, &context.scope, &context.repo_root)
        .map_err(CollectorError::Adapter)?;
    let visible_paths: BTreeSet<&str> = visible
        .iter()
        .map(|member| member.import_path.as_str())
        .collect();
    let by_import_path = members
        .iter()
        .map(|member| (member.import_path.as_str(), member))
        .collect::<BTreeMap<&str, &GoMember>>();

    let mut edges = BTreeSet::<(String, String)>::new();
    for package in &packages {
        if !visible_paths.contains(package.import_path.as_str()) {
            continue;
        }
        let Some(from_member) = by_import_path.get(package.import_path.as_str()) else {
            continue;
        };
        for dependency in &package.imports {
            let Some(to_member) = by_import_path.get(dependency.as_str()) else {
                continue;
            };
            edges.insert((from_member.dir.clone(), to_member.dir.clone()));
        }
    }

    let compiled_rules = compile_rules(&rules).map_err(CollectorError::Adapter)?;
    let mut offenders = Vec::new();
    for (from, to) in &edges {
        for rule in &compiled_rules {
            if rule.from.matches(from) && rule.to.matches(to) {
                offenders.push(DepsOffender {
                    from: from.clone(),
                    to: to.clone(),
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
            .then_with(|| left.rule.cmp(&right.rule))
    });

    Ok(SignalRow {
        kind: SignalKind::Deps,
        language: Language::Go,
        scope: Scope {
            workspace_root: context.scope.workspace_root.clone(),
            path: context.scope.path.clone(),
            package: context.scope.package.clone(),
            file: context.scope.file.clone(),
        },
        pass: offenders.is_empty(),
        result: SignalResult::Deps(DepsResult {
            crate_count: visible.len() as u64,
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
        language: Language::Go,
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
            violation_count: 0,
            failure: Some(failure),
        }),
        budget: Budget::Deps(DepsBudget {
            forbidden: context
                .policy
                .go
                .deps
                .as_ref()
                .map(|value| value.forbidden.clone()),
        }),
        offenders: Offenders::Deps(Vec::new()),
    }
}

fn visible_members<'a>(
    members: &'a [GoMember],
    scope: &Scope,
    repo_root: &Path,
) -> Result<Vec<&'a GoMember>, String> {
    if let Some(package) = &scope.package {
        let member = members
            .iter()
            .find(|member| member.import_path == *package || member.dir == *package)
            .ok_or_else(|| format!("package scope '{package}' was not found in go packages"))?;
        return Ok(vec![member]);
    }

    let target = if let Some(file) = &scope.file {
        Some(resolve_repo_path(repo_root, file))
    } else {
        scope
            .path
            .as_ref()
            .map(|path| resolve_repo_path(repo_root, path))
    };
    let Some(target) = target else {
        return Ok(members.iter().collect());
    };
    let target = target.canonicalize().map_err(|error| {
        format!(
            "dependency scope {} could not be resolved: {error}",
            target.display()
        )
    })?;

    Ok(members
        .iter()
        .filter(|member| {
            let absolute = repo_root.join(&member.dir);
            target.starts_with(&absolute) || absolute.starts_with(&target)
        })
        .collect())
}

struct CompiledRule {
    from_raw: String,
    to_raw: String,
    from: Pattern,
    to: Pattern,
}

fn compile_rules(forbidden: &BTreeMap<String, Vec<String>>) -> Result<Vec<CompiledRule>, String> {
    let mut compiled = Vec::new();
    for (from, tos) in forbidden {
        let from_pattern = Pattern::new(from)
            .map_err(|error| format!("invalid forbidden deps pattern '{from}': {error}"))?;
        for to in tos {
            compiled.push(CompiledRule {
                from_raw: from.clone(),
                to_raw: to.clone(),
                from: from_pattern.clone(),
                to: Pattern::new(to)
                    .map_err(|error| format!("invalid forbidden deps pattern '{to}': {error}"))?,
            });
        }
    }
    Ok(compiled)
}
