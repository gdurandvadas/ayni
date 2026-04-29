use super::util::{resolve_repo_path, to_repo_relative_path};
use ayni_core::{
    Budget, DepsOffender, DepsResult, Language, Level, Offenders, RunContext, Scope, SignalKind,
    SignalResult, SignalRow,
};
use glob::Pattern;
use serde::Deserialize;
use serde_json::json;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

pub fn collect(context: &RunContext) -> Result<SignalRow, String> {
    let rules = context
        .policy
        .node
        .deps
        .as_ref()
        .map(|value| value.forbidden.clone())
        .unwrap_or_default();
    let workspace = NodeWorkspace::load(&context.workdir, &context.repo_root)?;
    let visible = workspace.visible_members(&context.scope, &context.repo_root)?;
    let member_by_name = workspace
        .members
        .iter()
        .map(|member| (member.name.as_str(), member))
        .collect::<BTreeMap<&str, &NodeMember>>();

    let mut edges = std::collections::BTreeSet::<(String, String)>::new();
    for source in &visible {
        for dependency in source.declared_workspace_deps(&member_by_name) {
            if let Some(target) = member_by_name.get(dependency.as_str()) {
                edges.insert((source.dir.clone(), target.dir.clone()));
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
        language: Language::Node,
        scope: Scope {
            workspace_root: context.scope.workspace_root.clone(),
            path: context.scope.path.clone(),
            package: context.scope.package.clone(),
            file: context.scope.file.clone(),
        },
        pass: offenders.is_empty(),
        result: SignalResult::Deps(DepsResult {
            crate_count: workspace.members.len() as u64,
            edge_count: edges.len() as u64,
            violation_count: offenders.len() as u64,
        }),
        budget: Budget::Deps(json!({ "forbidden": rules })),
        offenders: Offenders::Deps(offenders),
        delta_vs_previous: None,
        delta_vs_baseline: None,
    })
}

#[derive(Debug, Clone, Deserialize)]
struct NodePackage {
    name: Option<String>,
    workspaces: Option<WorkspacesField>,
    dependencies: Option<BTreeMap<String, String>>,
    #[serde(rename = "devDependencies")]
    dev_dependencies: Option<BTreeMap<String, String>>,
    #[serde(rename = "peerDependencies")]
    peer_dependencies: Option<BTreeMap<String, String>>,
    #[serde(rename = "optionalDependencies")]
    optional_dependencies: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum WorkspacesField {
    Simple(Vec<String>),
    Structured { packages: Vec<String> },
}

#[derive(Debug, Clone)]
struct NodeMember {
    name: String,
    dir: String,
    package: NodePackage,
}

impl NodeMember {
    fn declared_workspace_deps(
        &self,
        members: &BTreeMap<&str, &NodeMember>,
    ) -> std::collections::BTreeSet<String> {
        let mut deps = std::collections::BTreeSet::new();
        for section in [
            self.package.dependencies.as_ref(),
            self.package.dev_dependencies.as_ref(),
            self.package.peer_dependencies.as_ref(),
            self.package.optional_dependencies.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            for name in section.keys() {
                if members.contains_key(name.as_str()) {
                    deps.insert(name.clone());
                }
            }
        }
        deps
    }
}

struct NodeWorkspace {
    members: Vec<NodeMember>,
}

impl NodeWorkspace {
    fn load(root: &Path, repo_root: &Path) -> Result<Self, String> {
        let root_manifest = parse_node_package(&root.join("package.json"))?;
        let root_name = root_manifest
            .name
            .clone()
            .unwrap_or_else(|| to_repo_relative_path(repo_root, root));
        let root_dir = to_repo_relative_path(repo_root, root);
        let mut members = vec![NodeMember {
            name: root_name,
            dir: root_dir,
            package: root_manifest,
        }];

        let workspace_patterns = members
            .first()
            .and_then(|member| member.package.workspaces.as_ref())
            .map(workspace_patterns)
            .unwrap_or_default();
        for pattern in workspace_patterns {
            for member_dir in expand_workspace_pattern(root, &pattern)? {
                let manifest = parse_node_package(&member_dir.join("package.json"))?;
                let Some(name) = manifest.name.clone() else {
                    continue;
                };
                let dir = to_repo_relative_path(repo_root, &member_dir);
                members.push(NodeMember {
                    name,
                    dir,
                    package: manifest,
                });
            }
        }
        members.sort_by(|left, right| left.name.cmp(&right.name));
        members.dedup_by(|left, right| left.name == right.name);
        Ok(Self { members })
    }

    fn visible_members<'a>(
        &'a self,
        scope: &Scope,
        repo_root: &Path,
    ) -> Result<Vec<&'a NodeMember>, String> {
        if let Some(package) = &scope.package {
            let member = self
                .members
                .iter()
                .find(|member| member.name == *package || member.dir == *package)
                .ok_or_else(|| {
                    format!("package scope '{package}' was not found in node workspace")
                })?;
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
            return Ok(self.members.iter().collect());
        };
        let target = target.canonicalize().map_err(|error| {
            format!(
                "dependency scope {} could not be resolved: {error}",
                target.display()
            )
        })?;
        Ok(self
            .members
            .iter()
            .filter(|member| {
                let member_abs = repo_root.join(&member.dir);
                target.starts_with(&member_abs) || member_abs.starts_with(&target)
            })
            .collect())
    }
}

fn parse_node_package(path: &Path) -> Result<NodePackage, String> {
    let content = fs::read_to_string(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    serde_json::from_str::<NodePackage>(&content)
        .map_err(|error| format!("failed to parse {}: {error}", path.display()))
}

fn workspace_patterns(field: &WorkspacesField) -> Vec<String> {
    match field {
        WorkspacesField::Simple(items) => items.clone(),
        WorkspacesField::Structured { packages } => packages.clone(),
    }
}

fn expand_workspace_pattern(root: &Path, pattern: &str) -> Result<Vec<PathBuf>, String> {
    if !pattern.ends_with("/*") {
        let direct = root.join(pattern);
        if direct.join("package.json").is_file() {
            return Ok(vec![direct]);
        }
        return Ok(Vec::new());
    }
    let base = root.join(pattern.trim_end_matches("/*"));
    let mut members = Vec::new();
    let Ok(entries) = fs::read_dir(base) else {
        return Ok(Vec::new());
    };
    for entry in entries.flatten() {
        let candidate = entry.path();
        if candidate.is_dir() && candidate.join("package.json").is_file() {
            members.push(candidate);
        }
    }
    Ok(members)
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
