use super::util::to_repo_relative_path;
use ayni_core::{
    Budget, DepsOffender, DepsResult, Language, Level, Offenders, RunContext, Scope, SignalKind,
    SignalResult, SignalRow,
};
use glob::Pattern;
use regex::Regex;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

pub fn collect(context: &RunContext) -> Result<SignalRow, String> {
    let rules = context
        .policy
        .python
        .deps
        .as_ref()
        .map(|value| value.forbidden.clone())
        .unwrap_or_default();
    let files = python_files(context);
    let modules = module_index(context, &files);
    let mut edges = BTreeSet::<(String, String)>::new();
    for file in &files {
        let imports = imports_in_file(file)?;
        let from = to_repo_relative_path(&context.repo_root, file);
        for import in imports {
            if let Some(target) = resolve_import(&modules, &import)
                && target != from
            {
                edges.insert((from.clone(), target));
            }
        }
    }

    let compiled_rules = compile_rules(&rules)?;
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
    });

    Ok(SignalRow {
        kind: SignalKind::Deps,
        language: Language::Python,
        scope: Scope {
            workspace_root: context.scope.workspace_root.clone(),
            path: context.scope.path.clone(),
            package: context.scope.package.clone(),
            file: context.scope.file.clone(),
        },
        pass: offenders.is_empty(),
        result: SignalResult::Deps(DepsResult {
            crate_count: files.len() as u64,
            edge_count: edges.len() as u64,
            violation_count: offenders.len() as u64,
        }),
        budget: Budget::Deps(json!({ "forbidden": rules })),
        offenders: Offenders::Deps(offenders),
        delta_vs_previous: None,
        delta_vs_baseline: None,
    })
}

fn python_files(context: &RunContext) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for entry in WalkDir::new(&context.workdir)
        .into_iter()
        .filter_entry(|entry| !is_excluded_dir(entry.path()))
    {
        let Ok(entry) = entry else {
            continue;
        };
        let path = entry.path();
        if entry.file_type().is_file() && path.extension().and_then(|v| v.to_str()) == Some("py") {
            files.push(path.to_path_buf());
        }
    }
    files.sort();
    files
}

fn is_excluded_dir(path: &Path) -> bool {
    matches!(
        path.file_name().and_then(|value| value.to_str()),
        Some(".venv" | "venv" | "env" | "__pycache__" | ".tox" | ".nox" | ".git" | ".ayni")
    )
}

fn module_index(context: &RunContext, files: &[PathBuf]) -> BTreeMap<String, String> {
    let mut modules = BTreeMap::new();
    for file in files {
        let rel = to_repo_relative_path(&context.workdir, file);
        let repo_rel = to_repo_relative_path(&context.repo_root, file);
        let Some(module) = module_name_from_rel(&rel) else {
            continue;
        };
        modules.insert(module, repo_rel);
    }
    modules
}

fn module_name_from_rel(rel: &str) -> Option<String> {
    let without_ext = rel.strip_suffix(".py")?;
    let without_init = without_ext.strip_suffix("/__init__").unwrap_or(without_ext);
    Some(without_init.replace('/', "."))
}

fn imports_in_file(path: &Path) -> Result<Vec<String>, String> {
    let content = fs::read_to_string(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    extract_imports(&content)
}

fn extract_imports(content: &str) -> Result<Vec<String>, String> {
    let import_re = Regex::new(r"^\s*import\s+(.+)$")
        .map_err(|error| format!("failed to compile import regex: {error}"))?;
    let from_re = Regex::new(r"^\s*from\s+([A-Za-z_][\w\.]*)\s+import\s+")
        .map_err(|error| format!("failed to compile from-import regex: {error}"))?;
    let mut imports = Vec::new();
    for line in content.lines() {
        let line = line.split('#').next().unwrap_or("").trim_end();
        if let Some(caps) = import_re.captures(line) {
            let raw = caps.get(1).map(|value| value.as_str()).unwrap_or("");
            for item in raw.split(',') {
                let name = item
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if !name.is_empty() {
                    imports.push(name);
                }
            }
        } else if let Some(caps) = from_re.captures(line)
            && let Some(name) = caps.get(1)
        {
            imports.push(name.as_str().to_string());
        }
    }
    Ok(imports)
}

fn resolve_import(modules: &BTreeMap<String, String>, import: &str) -> Option<String> {
    let mut candidate = import;
    loop {
        if let Some(path) = modules.get(candidate) {
            return Some(path.clone());
        }
        let (prefix, _) = candidate.rsplit_once('.')?;
        candidate = prefix;
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
            let to_pattern = Pattern::new(target).map_err(|error| {
                format!("invalid deps forbidden target glob '{target}': {error}")
            })?;
            out.push(CompiledRule {
                from_raw: from.clone(),
                to_raw: target.clone(),
                from: from_pattern.clone(),
                to: to_pattern,
            });
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::{extract_imports, resolve_import};
    use std::collections::BTreeMap;

    #[test]
    fn extracts_imports() {
        let imports = extract_imports(
            r#"
import os, src.presentation.api as api
from src.domain import model
"#,
        )
        .expect("imports");
        assert_eq!(
            imports,
            vec![
                "os".to_string(),
                "src.presentation.api".to_string(),
                "src.domain".to_string()
            ]
        );
    }

    #[test]
    fn resolves_longest_internal_prefix() {
        let mut modules = BTreeMap::new();
        modules.insert(
            "src.presentation".to_string(),
            "src/presentation/__init__.py".to_string(),
        );
        modules.insert(
            "src.presentation.api".to_string(),
            "src/presentation/api.py".to_string(),
        );
        assert_eq!(
            resolve_import(&modules, "src.presentation.api.handlers"),
            Some("src/presentation/api.py".to_string())
        );
    }
}
