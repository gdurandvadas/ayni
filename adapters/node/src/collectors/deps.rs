use crate::workspace::WorkspacePatterns;
use ayni_adapters_common::discovery::discover_file_parent_roots;
use ayni_adapters_common::paths::{
    canonicalize_relative_posix, resolve_repo_path, to_repo_relative_path,
};
use ayni_core::{
    Budget, DepsBudget, DepsOffender, DepsResult, Language, Level, Offenders, RunContext, Scope,
    SignalKind, SignalResult, SignalRow,
};
use glob::Pattern;
use serde::Deserialize;
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
    let workspace_root = governing_workspace_root(context)?;
    let workspace = NodeWorkspace::load(&workspace_root, &context.repo_root)?;
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

fn governing_workspace_root(context: &RunContext) -> Result<PathBuf, String> {
    let mut current = context.target_root.as_path();
    loop {
        let manifest = current.join("package.json");
        if manifest.is_file() {
            let value = parse_manifest_value(&manifest)?;
            let patterns = WorkspacePatterns::parse(&value, &manifest)?;
            let relative = if context.target_root == current {
                String::from(".")
            } else {
                context
                    .target_root
                    .strip_prefix(current)
                    .map(|path| canonicalize_relative_posix(&path.to_string_lossy()))
                    .unwrap_or_default()
            };
            if !patterns.is_empty() && (relative == "." || patterns.matches(&relative)) {
                return Ok(current.to_path_buf());
            }
        }
        if current == context.repo_root {
            return Ok(context.workdir.clone());
        }
        let Some(parent) = current
            .parent()
            .filter(|parent| parent.starts_with(&context.repo_root))
        else {
            return Ok(context.workdir.clone());
        };
        current = parent;
    }
}

#[derive(Debug, Clone, Deserialize)]
struct NodePackage {
    name: Option<String>,
    dependencies: Option<BTreeMap<String, String>>,
    #[serde(rename = "devDependencies")]
    dev_dependencies: Option<BTreeMap<String, String>>,
    #[serde(rename = "peerDependencies")]
    peer_dependencies: Option<BTreeMap<String, String>>,
    #[serde(rename = "optionalDependencies")]
    optional_dependencies: Option<BTreeMap<String, String>>,
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
        let root_manifest_path = root.join("package.json");
        let root_value = parse_manifest_value(&root_manifest_path)?;
        let patterns = WorkspacePatterns::parse(&root_value, &root_manifest_path)?;
        let root_manifest = parse_node_package(&root_manifest_path)?;
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

        for relative in discover_file_parent_roots(root, "package.json", |parts| {
            parts
                .iter()
                .any(|part| matches!(*part, "node_modules" | ".git" | ".ayni"))
        }) {
            if relative == "." || !patterns.matches(&relative) {
                continue;
            }
            let member_dir = root.join(&relative);
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

fn parse_manifest_value(path: &Path) -> Result<serde_json::Value, String> {
    let content = fs::read_to_string(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    serde_json::from_str(&content)
        .map_err(|error| format!("failed to parse {}: {error}", path.display()))
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
    use super::collect;
    use ayni_core::{AyniPolicy, ExecutionResolution, RunContext, Scope, SignalResult};

    #[test]
    fn general_and_negated_patterns_define_dependency_membership() {
        let directory = tempfile::tempdir().expect("fixture");
        let root = directory.path();
        std::fs::write(
            root.join("package.json"),
            r#"{"name":"root","workspaces":["packages/**","!packages/excluded/**"]}"#,
        )
        .expect("workspace");
        for (path, name, dependencies) in [
            ("packages/base", "base", "{}"),
            ("packages/app", "app", r#"{"base":"*"}"#),
            ("packages/excluded/tool", "excluded", r#"{"base":"*"}"#),
        ] {
            let package = root.join(path);
            std::fs::create_dir_all(&package).expect("package");
            std::fs::write(
                package.join("package.json"),
                format!(r#"{{"name":"{name}","dependencies":{dependencies}}}"#),
            )
            .expect("manifest");
        }
        let canonical = root.canonicalize().expect("canonical");
        let context = RunContext {
            repo_root: canonical.clone(),
            target_root: canonical.clone(),
            workdir: canonical.clone(),
            policy: AyniPolicy::default(),
            scope: Scope::default(),
            execution: ExecutionResolution::direct("npm", canonical, "test", 100),
            debug: false,
        };

        let row = collect(&context).expect("dependency row");
        let SignalResult::Deps(result) = row.result else {
            panic!("deps result");
        };
        assert_eq!(result.crate_count, 3);
        assert_eq!(result.edge_count, 1);
    }

    #[test]
    fn member_target_can_collect_a_reverse_dependent_package_from_workspace() {
        let directory = tempfile::tempdir().expect("fixture");
        let root = directory.path();
        std::fs::write(
            root.join("package.json"),
            r#"{"name":"root","workspaces":["packages/*"]}"#,
        )
        .expect("workspace");
        for (name, dependencies) in [("base", "{}"), ("app", r#"{"base":"*"}"#)] {
            let package = root.join("packages").join(name);
            std::fs::create_dir_all(&package).expect("package");
            std::fs::write(
                package.join("package.json"),
                format!(r#"{{"name":"{name}","dependencies":{dependencies}}}"#),
            )
            .expect("manifest");
        }
        let canonical = root.canonicalize().expect("canonical");
        let target = canonical.join("packages/base");
        let context = RunContext {
            repo_root: canonical.clone(),
            target_root: target.clone(),
            workdir: target.clone(),
            policy: AyniPolicy::default(),
            scope: Scope {
                workspace_root: canonical.to_string_lossy().into_owned(),
                path: Some(String::from("packages/base")),
                package: Some(String::from("app")),
                file: None,
            },
            execution: ExecutionResolution::direct("npm", target, "test", 100),
            debug: false,
        };

        let row = collect(&context).expect("dependency row");

        assert!(row.pass);
        assert_eq!(row.scope.package.as_deref(), Some("app"));
    }
}
