use super::util::to_repo_relative_path;
use ayni_core::{
    Budget, Language, Level, Offenders, RunContext, Scope, SignalKind, SignalResult, SignalRow,
    SizeOffender, SizeResult, SizeThreshold,
};
use glob::Pattern;
use serde_json::json;
use std::collections::BTreeMap;
use std::fs;
use walkdir::WalkDir;

pub fn collect(context: &RunContext) -> Result<SignalRow, String> {
    let size_map = context.policy.size_rules_for(Language::Go);
    if size_map.is_empty() {
        return Err(String::from(
            "missing size config: add [go.size] with at least one glob entry to .ayni.toml",
        ));
    }
    let compiled = compile_rules(size_map)?;

    let mut offenders = Vec::new();
    let mut max_lines = 0_u64;
    let mut warn_count = 0_u64;
    let mut fail_count = 0_u64;
    let mut total_files = 0_u64;

    for entry in WalkDir::new(&context.workdir) {
        let entry = match entry {
            Ok(value) => value,
            Err(_) => continue,
        };
        if !entry.file_type().is_file() {
            continue;
        }
        let rel = to_repo_relative_path(&context.repo_root, entry.path());
        let Some(threshold) = first_matching(&compiled, &rel) else {
            continue;
        };

        total_files += 1;
        let content = fs::read_to_string(entry.path())
            .map_err(|error| format!("failed to read {}: {error}", entry.path().display()))?;
        let line_count = content.lines().count() as u64;
        max_lines = max_lines.max(line_count);

        if line_count > threshold.warn {
            let level = if line_count > threshold.fail {
                fail_count += 1;
                Level::Fail
            } else {
                warn_count += 1;
                Level::Warn
            };
            offenders.push(SizeOffender {
                file: rel,
                value: line_count,
                warn: threshold.warn,
                fail: threshold.fail,
                level,
            });
        }
    }

    let budget_rules: Vec<_> = size_map
        .iter()
        .map(|(glob, t)| json!({ "glob": glob, "warn": t.warn, "fail": t.fail }))
        .collect();

    Ok(SignalRow {
        kind: SignalKind::Size,
        language: Language::Go,
        scope: Scope {
            workspace_root: context.scope.workspace_root.clone(),
            path: context.scope.path.clone(),
            package: context.scope.package.clone(),
            file: context.scope.file.clone(),
        },
        pass: fail_count == 0,
        result: SignalResult::Size(SizeResult {
            max_lines,
            total_files,
            warn_count,
            fail_count,
        }),
        budget: Budget::Size(json!({ "rules": budget_rules })),
        offenders: Offenders::Size(offenders),
        delta_vs_previous: None,
        delta_vs_baseline: None,
    })
}

struct CompiledRule<'a> {
    threshold: &'a SizeThreshold,
    include: Pattern,
    excludes: Vec<Pattern>,
}

fn compile_rules(map: &BTreeMap<String, SizeThreshold>) -> Result<Vec<CompiledRule<'_>>, String> {
    map.iter()
        .map(|(glob, threshold)| {
            let include = Pattern::new(glob)
                .map_err(|error| format!("invalid size glob '{glob}': {error}"))?;
            let excludes = threshold
                .exclude
                .iter()
                .map(|ex| {
                    Pattern::new(ex)
                        .map_err(|error| format!("invalid exclude glob '{ex}': {error}"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(CompiledRule {
                threshold,
                include,
                excludes,
            })
        })
        .collect()
}

fn first_matching<'a>(compiled: &[CompiledRule<'a>], rel: &str) -> Option<&'a SizeThreshold> {
    compiled
        .iter()
        .find(|rule| rule.include.matches(rel) && !rule.excludes.iter().any(|ex| ex.matches(rel)))
        .map(|rule| rule.threshold)
}
