use ayni_core::{
    Budget, DepsOffender, DepsResult, Language, Level, Offenders, RunContext, Scope, SignalKind,
    SignalResult, SignalRow,
};
use glob::Pattern;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn collect(context: &RunContext) -> Result<SignalRow, String> {
    let rules = context
        .policy
        .rust
        .deps
        .as_ref()
        .map(|value| value.forbidden.clone())
        .unwrap_or_default();

    let metadata = load_metadata(&context.workdir)?;
    let analysis = analyze_deps(
        &metadata,
        &context.repo_root,
        &context.scope,
        &context.workdir,
        &rules,
    )?;

    Ok(SignalRow {
        kind: SignalKind::Deps,
        language: Language::Rust,
        scope: Scope {
            workspace_root: context.scope.workspace_root.clone(),
            path: context.scope.path.clone(),
            package: context.scope.package.clone(),
            file: context.scope.file.clone(),
        },
        pass: analysis.result.violation_count == 0,
        result: SignalResult::Deps(analysis.result),
        budget: Budget::Deps(json!({ "forbidden": rules })),
        offenders: Offenders::Deps(analysis.offenders),
        delta_vs_previous: None,
        delta_vs_baseline: None,
    })
}

#[derive(Debug, serde::Deserialize)]
struct CargoMetadata {
    packages: Vec<MetadataPackage>,
    workspace_members: Vec<String>,
    resolve: Option<MetadataResolve>,
}

#[derive(Debug, serde::Deserialize)]
struct MetadataPackage {
    id: String,
    name: String,
    manifest_path: String,
}

#[derive(Debug, serde::Deserialize)]
struct MetadataResolve {
    nodes: Vec<MetadataNode>,
}

#[derive(Debug, serde::Deserialize)]
struct MetadataNode {
    id: String,
    deps: Vec<MetadataDep>,
}

#[derive(Debug, serde::Deserialize)]
struct MetadataDep {
    pkg: String,
}

#[derive(Debug, Clone)]
struct MemberInfo {
    id: String,
    name: String,
    dir: String,
    dir_abs: PathBuf,
}

struct DepsAnalysis {
    result: DepsResult,
    offenders: Vec<DepsOffender>,
}

fn load_metadata(repo_root: &Path) -> Result<CargoMetadata, String> {
    let output = Command::new("cargo")
        .arg("metadata")
        .arg("--format-version")
        .arg("1")
        .current_dir(repo_root)
        .output()
        .map_err(|error| format!("failed to execute cargo metadata: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "cargo metadata failed (exit {}): {}",
            output.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("failed to parse cargo metadata output: {error}"))
}

fn analyze_deps(
    metadata: &CargoMetadata,
    repo_root: &Path,
    scope: &Scope,
    workdir: &Path,
    forbidden: &BTreeMap<String, Vec<String>>,
) -> Result<DepsAnalysis, String> {
    let canonical_repo_root = repo_root.canonicalize().map_err(|error| {
        format!(
            "failed to canonicalize repo root {}: {error}",
            repo_root.display()
        )
    })?;
    let members = workspace_members(metadata, &canonical_repo_root)?;
    let visible_members = visible_members(&members, scope, &canonical_repo_root)?;
    let canonical_workdir = workdir.canonicalize().map_err(|error| {
        format!(
            "failed to canonicalize workdir {}: {error}",
            workdir.display()
        )
    })?;
    let members_in_root: Vec<&MemberInfo> = members
        .iter()
        .filter(|member| {
            member.dir_abs == canonical_workdir || member.dir_abs.starts_with(&canonical_workdir)
        })
        .collect();
    let root_member_ids: BTreeSet<&str> = members_in_root
        .iter()
        .map(|member| member.id.as_str())
        .collect();
    let visible_members: Vec<&MemberInfo> = visible_members
        .into_iter()
        .filter(|member| root_member_ids.contains(member.id.as_str()))
        .collect();
    let visible_ids: BTreeSet<&str> = visible_members
        .iter()
        .map(|member| member.id.as_str())
        .collect();
    let member_by_id: BTreeMap<&str, &MemberInfo> = members
        .iter()
        .map(|member| (member.id.as_str(), member))
        .collect();

    // Scope behavior is deterministic:
    // - root scope: all workspace members are visible
    // - package scope: only that member is visible
    // - file/path scope: members that contain the selected path, or live under the selected
    //   directory, are visible
    //
    // Deps scoping intentionally keeps all outgoing workspace-member edges from visible
    // sources, even when the destination member is outside the visible set. This prevents
    // scoped runs from hiding forbidden edges that cross the visibility boundary.
    //
    // Invariants:
    // - crate_count: number of visible source members
    // - edge_count: unique workspace-member edges considered from visible sources
    // - offenders: violations among considered edges
    let mut edges = BTreeSet::<(String, String)>::new();
    if let Some(resolve) = &metadata.resolve {
        for node in &resolve.nodes {
            if !visible_ids.contains(node.id.as_str()) {
                continue;
            }
            let Some(from_member) = member_by_id.get(node.id.as_str()) else {
                continue;
            };
            for dep in &node.deps {
                // Only keep edges to other workspace members; external dependencies are not
                // part of this collector's graph.
                let Some(to_member) = member_by_id.get(dep.pkg.as_str()) else {
                    continue;
                };
                if !root_member_ids.contains(to_member.id.as_str()) {
                    continue;
                }
                edges.insert((from_member.dir.clone(), to_member.dir.clone()));
            }
        }
    }

    let compiled_rules = compile_rules(forbidden)?;
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

    Ok(DepsAnalysis {
        result: DepsResult {
            crate_count: visible_members.len() as u64,
            edge_count: edges.len() as u64,
            violation_count: offenders.len() as u64,
        },
        offenders,
    })
}

fn workspace_members(
    metadata: &CargoMetadata,
    repo_root: &Path,
) -> Result<Vec<MemberInfo>, String> {
    let workspace_member_ids: BTreeSet<&str> = metadata
        .workspace_members
        .iter()
        .map(String::as_str)
        .collect();
    metadata
        .packages
        .iter()
        .filter(|package| workspace_member_ids.contains(package.id.as_str()))
        .map(|package| {
            let manifest = PathBuf::from(&package.manifest_path);
            let dir_abs = manifest
                .parent()
                .map(Path::to_path_buf)
                .ok_or_else(|| format!("package {} has no manifest parent", package.name))?;
            let dir_abs = dir_abs.canonicalize().map_err(|error| {
                format!(
                    "failed to canonicalize workspace member directory {}: {error}",
                    dir_abs.display()
                )
            })?;
            let dir = repo_relative_dir(repo_root, &dir_abs)?;
            Ok(MemberInfo {
                id: package.id.clone(),
                name: package.name.clone(),
                dir,
                dir_abs,
            })
        })
        .collect()
}

fn repo_relative_dir(repo_root: &Path, dir_abs: &Path) -> Result<String, String> {
    let relative = dir_abs.strip_prefix(repo_root).map_err(|error| {
        format!(
            "workspace member {} is outside repo root {}: {error}",
            dir_abs.display(),
            repo_root.display()
        )
    })?;
    let text = relative.to_string_lossy().replace('\\', "/");
    Ok(if text.is_empty() {
        String::from(".")
    } else {
        text
    })
}

fn visible_members<'a>(
    members: &'a [MemberInfo],
    scope: &Scope,
    repo_root: &Path,
) -> Result<Vec<&'a MemberInfo>, String> {
    if let Some(package) = &scope.package {
        let member = members
            .iter()
            .find(|member| member.name == *package || member.dir == *package)
            .ok_or_else(|| format!("package scope '{package}' was not found in cargo metadata"))?;
        return Ok(vec![member]);
    }

    let Some(target) = scoped_path(scope, repo_root)? else {
        return Ok(members.iter().collect());
    };
    Ok(members
        .iter()
        .filter(|member| target.starts_with(&member.dir_abs) || member.dir_abs.starts_with(&target))
        .collect())
}

fn scoped_path(scope: &Scope, repo_root: &Path) -> Result<Option<PathBuf>, String> {
    let value = if let Some(file) = &scope.file {
        Some(file.as_str())
    } else {
        scope.path.as_deref()
    };
    let Some(value) = value else {
        return Ok(None);
    };
    let path = if Path::new(value).is_absolute() {
        PathBuf::from(value)
    } else {
        repo_root.join(value)
    };
    path.canonicalize().map(Some).map_err(|error| {
        format!(
            "dependency scope {} could not be resolved: {error}",
            path.display()
        )
    })
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

#[cfg(test)]
mod tests {
    use super::{CargoMetadata, analyze_deps};
    use ayni_core::Scope;
    use std::collections::BTreeMap;
    use std::fs;
    use tempfile::TempDir;

    fn workspace_fixture_metadata(root: &std::path::Path) -> CargoMetadata {
        let metadata_json = format!(
            r#"{{
                "packages": [
                    {{"id": "core 0.1.0", "name": "ayni-core", "manifest_path": "{}/core/Cargo.toml"}},
                    {{"id": "adapters-rust 0.1.0", "name": "ayni-adapters-rust", "manifest_path": "{}/adapters/rust/Cargo.toml"}},
                    {{"id": "cli 0.1.0", "name": "ayni-cli", "manifest_path": "{}/cli/Cargo.toml"}}
                ],
                "workspace_members": ["core 0.1.0", "adapters-rust 0.1.0", "cli 0.1.0"],
                "resolve": {{
                    "nodes": [
                        {{"id": "core 0.1.0", "deps": [{{"pkg": "adapters-rust 0.1.0"}}]}},
                        {{"id": "adapters-rust 0.1.0", "deps": [{{"pkg": "cli 0.1.0"}}]}},
                        {{"id": "cli 0.1.0", "deps": []}}
                    ]
                }}
            }}"#,
            root.display(),
            root.display(),
            root.display()
        );
        serde_json::from_str(&metadata_json).expect("metadata parse")
    }

    #[test]
    fn deps_rule_matching_uses_repo_relative_member_dirs() {
        let temp = TempDir::new().expect("tempdir");
        let root = temp.path();
        fs::create_dir_all(root.join("core")).expect("core dir");
        fs::create_dir_all(root.join("adapters/rust")).expect("adapter dir");
        fs::create_dir_all(root.join("cli")).expect("cli dir");

        let metadata = workspace_fixture_metadata(root);

        let forbidden = BTreeMap::from([
            (String::from("core"), vec![String::from("adapters/*")]),
            (String::from("adapters/*"), vec![String::from("cli")]),
        ]);
        let analysis =
            analyze_deps(&metadata, root, &Scope::default(), root, &forbidden).expect("analysis");

        assert_eq!(analysis.result.crate_count, 3);
        assert_eq!(analysis.result.edge_count, 2);
        assert_eq!(analysis.result.violation_count, 2);
        assert_eq!(analysis.offenders[0].from, "adapters/rust");
        assert_eq!(analysis.offenders[0].to, "cli");
        assert_eq!(analysis.offenders[0].rule, "adapters/* -> cli");
        assert_eq!(analysis.offenders[1].rule, "core -> adapters/*");
    }

    #[test]
    fn scoped_package_keeps_outgoing_edges_to_non_visible_members() {
        let temp = TempDir::new().expect("tempdir");
        let root = temp.path();
        fs::create_dir_all(root.join("core")).expect("core dir");
        fs::create_dir_all(root.join("adapters/rust")).expect("adapter dir");
        fs::create_dir_all(root.join("cli")).expect("cli dir");

        let metadata = workspace_fixture_metadata(root);

        let forbidden = BTreeMap::from([(String::from("core"), vec![String::from("adapters/*")])]);
        let analysis = analyze_deps(
            &metadata,
            root,
            &Scope {
                package: Some(String::from("ayni-core")),
                ..Scope::default()
            },
            root,
            &forbidden,
        )
        .expect("analysis");

        assert_eq!(analysis.result.crate_count, 1);
        assert_eq!(analysis.result.edge_count, 1);
        assert_eq!(analysis.result.violation_count, 1);
        assert_eq!(analysis.offenders.len(), 1);
        assert_eq!(analysis.offenders[0].from, "core");
        assert_eq!(analysis.offenders[0].to, "adapters/rust");
        assert_eq!(analysis.offenders[0].rule, "core -> adapters/*");
    }
}
