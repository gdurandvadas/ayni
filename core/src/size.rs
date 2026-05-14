use crate::SizeThreshold;
use crate::signal::{Level, SizeOffender, SizeResult};
use glob::Pattern;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use walkdir::WalkDir;

struct CompiledRule<'a> {
    threshold: &'a SizeThreshold,
    include: Pattern,
    excludes: Vec<Pattern>,
}

pub struct SizeCollection {
    pub result: SizeResult,
    pub offenders: Vec<SizeOffender>,
    pub budget: Value,
}

pub fn collect_size(
    repo_root: &Path,
    workdir: &Path,
    size_map: &BTreeMap<String, SizeThreshold>,
    excluded_dir_names: &[&str],
) -> Result<SizeCollection, String> {
    let compiled = compile_rules(size_map)?;
    let mut offenders = Vec::new();
    let mut max_lines = 0_u64;
    let mut warn_count = 0_u64;
    let mut fail_count = 0_u64;
    let mut total_files = 0_u64;

    for entry in WalkDir::new(workdir)
        .into_iter()
        .filter_entry(|entry| !is_excluded_dir(entry.path(), excluded_dir_names))
    {
        let entry = match entry {
            Ok(value) => value,
            Err(_) => continue,
        };
        if !entry.file_type().is_file() {
            continue;
        }

        let rel_for_match = to_repo_relative_path(workdir, entry.path());
        let Some(threshold) = first_matching(&compiled, &rel_for_match) else {
            continue;
        };

        let rel = to_repo_relative_path(repo_root, entry.path());
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

    Ok(SizeCollection {
        result: SizeResult {
            max_lines,
            total_files,
            warn_count,
            fail_count,
        },
        offenders,
        budget: json!({ "rules": budget_rules }),
    })
}

fn compile_rules(map: &BTreeMap<String, SizeThreshold>) -> Result<Vec<CompiledRule<'_>>, String> {
    map.iter()
        .map(|(glob, threshold)| {
            let include = Pattern::new(glob)
                .map_err(|error| format!("invalid size glob '{glob}': {error}"))?;
            let excludes = threshold
                .exclude
                .iter()
                .map(|exclude| {
                    Pattern::new(exclude)
                        .map_err(|error| format!("invalid exclude glob '{exclude}': {error}"))
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

fn is_excluded_dir(path: &Path, excluded_dir_names: &[&str]) -> bool {
    matches!(
        path.file_name().and_then(|value| value.to_str()),
        Some(name) if excluded_dir_names.contains(&name)
    )
}

fn to_repo_relative_path(repo_root: &Path, candidate: &Path) -> String {
    if let Ok(relative) = candidate.strip_prefix(repo_root) {
        return relative.to_string_lossy().replace('\\', "/");
    }
    if let Ok(canonical_repo_root) = repo_root.canonicalize()
        && let Ok(canonical_candidate) = candidate.canonicalize()
        && let Ok(relative) = canonical_candidate.strip_prefix(canonical_repo_root)
    {
        return relative.to_string_lossy().replace('\\', "/");
    }
    candidate.to_string_lossy().replace('\\', "/")
}
