use super::util::to_repo_relative_path;
use ayni_adapters_common::deps::{compile_rules, matching_offenders};
use ayni_core::{
    Budget, DepsBudget, DepsResult, Language, Offenders, RunContext, SignalKind, SignalResult,
    SignalRow,
};
use regex::Regex;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
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
    let offenders = matching_offenders(&edges, &compiled_rules);

    Ok(SignalRow {
        kind: SignalKind::Deps,
        language: Language::Python,
        scope: context.scope.clone(),
        pass: offenders.is_empty(),
        result: SignalResult::Deps(DepsResult {
            crate_count: files.len() as u64,
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
    Ok(extract_imports(&content))
}

fn extract_imports(content: &str) -> Vec<String> {
    static IMPORT_RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^\s*import\s+(.+)$").expect("valid import regex"));
    static FROM_RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^\s*from\s+([A-Za-z_][\w\.]*)\s+import\s+").expect("valid from-import regex")
    });
    let mut imports = Vec::new();
    for line in content.lines() {
        let line = line.split('#').next().unwrap_or("").trim_end();
        if let Some(caps) = IMPORT_RE.captures(line) {
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
        } else if let Some(caps) = FROM_RE.captures(line)
            && let Some(name) = caps.get(1)
        {
            imports.push(name.as_str().to_string());
        }
    }
    imports
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
        );
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
