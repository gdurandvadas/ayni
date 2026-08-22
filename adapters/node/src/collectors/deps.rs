use crate::workspace::WorkspacePatterns;
use ayni_adapters_common::paths::{
    canonicalize_relative_posix, resolve_repo_path, to_repo_relative_path,
};
use ayni_core::{
    Budget, DepsBudget, DepsOffender, DepsResult, Language, Level, Offenders, RunContext, Scope,
    SignalKind, SignalResult, SignalRow,
};
use glob::Pattern;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
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
    let canonical_repo_root = canonical_repository_root(&context.repo_root)?;
    let mut current = context.target_root.as_path();
    loop {
        let manifest = current.join("package.json");
        if manifest.is_file() {
            let value = parse_manifest_value(&canonical_repo_root, &manifest)?;
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
        let canonical_repo_root = canonical_repository_root(repo_root)?;
        let root_manifest_path = root.join("package.json");
        let root_value = parse_manifest_value(&canonical_repo_root, &root_manifest_path)?;
        let patterns = WorkspacePatterns::parse(&root_value, &root_manifest_path)?;
        let root_manifest = parse_node_package(&canonical_repo_root, &root_manifest_path)?;
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

        for relative in discover_manifest_parent_roots(root, &canonical_repo_root)? {
            if relative == "." || !patterns.matches(&relative) {
                continue;
            }
            let member_dir = root.join(&relative);
            let manifest =
                parse_node_package(&canonical_repo_root, &member_dir.join("package.json"))?;
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
        let canonical_repo_root = canonical_repository_root(repo_root)?;
        let canonical_members = self
            .members
            .iter()
            .map(|member| {
                let candidate = repo_root.join(&member.dir);
                let canonical = canonical_contained_path(
                    &canonical_repo_root,
                    &candidate,
                    "Node workspace member",
                )?;
                Ok((member, canonical))
            })
            .collect::<Result<Vec<_>, String>>()?;

        if let Some(package) = &scope.package {
            let member = canonical_members
                .iter()
                .find(|(member, _)| member.name == *package || member.dir == *package)
                .map(|(member, _)| *member)
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
            let visible = canonical_members
                .into_iter()
                .map(|(member, _)| member)
                .collect::<Vec<_>>();
            return require_non_package_members(scope, visible);
        };
        let target =
            canonical_contained_path(&canonical_repo_root, &target, "Node dependency scope")?;
        let visible = canonical_members
            .into_iter()
            .filter(|(_, member)| target.starts_with(member) || member.starts_with(&target))
            .map(|(member, _)| member)
            .collect::<Vec<_>>();
        require_non_package_members(scope, visible)
    }
}

fn require_non_package_members<'a>(
    scope: &Scope,
    visible: Vec<&'a NodeMember>,
) -> Result<Vec<&'a NodeMember>, String> {
    if scope.package.is_none() && visible.is_empty() {
        let requested_scope = scope
            .file
            .as_deref()
            .or(scope.path.as_deref())
            .unwrap_or(".");
        return Err(format!(
            "Node dependency analysis resolved no visible workspace members for non-package scope '{requested_scope}'"
        ));
    }
    Ok(visible)
}

fn discover_manifest_parent_roots(
    root: &Path,
    canonical_repo_root: &Path,
) -> Result<Vec<String>, String> {
    let mut found = BTreeSet::new();
    let mut stack = vec![(root.to_path_buf(), Vec::<PathBuf>::new())];

    while let Some((directory, ancestors)) = stack.pop() {
        visit_manifest_directory(
            root,
            canonical_repo_root,
            directory,
            ancestors,
            &mut found,
            &mut stack,
        )?;
    }

    Ok(found.into_iter().collect())
}

fn visit_manifest_directory(
    root: &Path,
    canonical_repo_root: &Path,
    directory: PathBuf,
    ancestors: Vec<PathBuf>,
    found: &mut BTreeSet<String>,
    stack: &mut Vec<(PathBuf, Vec<PathBuf>)>,
) -> Result<(), String> {
    let canonical_directory =
        canonical_contained_path(canonical_repo_root, &directory, "Node workspace directory")?;
    if ancestors.contains(&canonical_directory) {
        return Ok(());
    }
    let mut lineage = ancestors;
    lineage.push(canonical_directory);

    let mut children = Vec::new();
    for entry in sorted_workspace_entries(&directory)? {
        if let Some(child) =
            inspect_workspace_entry(root, canonical_repo_root, entry, &lineage, found)?
        {
            children.push(child);
        }
    }
    for child in children.into_iter().rev() {
        stack.push((child, lineage.clone()));
    }
    Ok(())
}

fn sorted_workspace_entries(directory: &Path) -> Result<Vec<fs::DirEntry>, String> {
    let mut entries = fs::read_dir(directory)
        .map_err(|error| {
            format!(
                "failed to inspect Node workspace directory {}: {error}",
                directory.display()
            )
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            format!(
                "failed to inspect Node workspace entry below {}: {error}",
                directory.display()
            )
        })?;
    entries.sort_by_key(fs::DirEntry::file_name);
    Ok(entries)
}

fn inspect_workspace_entry(
    root: &Path,
    canonical_repo_root: &Path,
    entry: fs::DirEntry,
    lineage: &[PathBuf],
    found: &mut BTreeSet<String>,
) -> Result<Option<PathBuf>, String> {
    let path = entry.path();
    let metadata = fs::symlink_metadata(&path).map_err(|error| {
        format!(
            "failed to inspect Node workspace path {}: {error}",
            path.display()
        )
    })?;
    if workspace_entry_is_directory(&path, &metadata)? {
        return inspect_workspace_directory(root, canonical_repo_root, path, lineage);
    }
    if path.file_name().and_then(|value| value.to_str()) == Some("package.json") {
        record_package_manifest(root, canonical_repo_root, &path, found)?;
    }
    Ok(None)
}

fn workspace_entry_is_directory(path: &Path, metadata: &fs::Metadata) -> Result<bool, String> {
    if !metadata.file_type().is_symlink() {
        return Ok(metadata.is_dir());
    }
    // Ignore unrelated dangling links as before. Relevant directory links are
    // containment-checked before descent, while package.json links are checked
    // when the manifest is recorded.
    match fs::metadata(path) {
        Ok(target) => Ok(target.is_dir()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!(
            "failed to resolve Node workspace symlink {}: {error}",
            path.display()
        )),
    }
}

fn inspect_workspace_directory(
    root: &Path,
    canonical_repo_root: &Path,
    path: PathBuf,
    lineage: &[PathBuf],
) -> Result<Option<PathBuf>, String> {
    let relative = repo_relative_workspace_path(root, &path, "Node workspace directory")?;
    if relative
        .split('/')
        .any(|part| matches!(part, "node_modules" | ".git" | ".ayni"))
    {
        return Ok(None);
    }
    let canonical =
        canonical_contained_path(canonical_repo_root, &path, "Node workspace directory")?;
    Ok((!lineage.contains(&canonical)).then_some(path))
}

fn record_package_manifest(
    root: &Path,
    canonical_repo_root: &Path,
    path: &Path,
    found: &mut BTreeSet<String>,
) -> Result<(), String> {
    canonical_contained_path(canonical_repo_root, path, "Node package manifest")?;
    let parent = path.parent().unwrap_or(root);
    found.insert(repo_relative_workspace_path(
        root,
        parent,
        "Node package manifest",
    )?);
    Ok(())
}

fn repo_relative_workspace_path(
    root: &Path,
    path: &Path,
    description: &str,
) -> Result<String, String> {
    let relative = path.strip_prefix(root).map_err(|_| {
        format!(
            "{description} {} is not below {}",
            path.display(),
            root.display()
        )
    })?;
    Ok(canonicalize_relative_posix(&relative.to_string_lossy()))
}

fn canonical_repository_root(repo_root: &Path) -> Result<PathBuf, String> {
    repo_root.canonicalize().map_err(|error| {
        format!(
            "failed to establish Node dependency repository root {}: {error}",
            repo_root.display()
        )
    })
}

fn canonical_contained_path(
    canonical_repo_root: &Path,
    path: &Path,
    description: &str,
) -> Result<PathBuf, String> {
    let canonical = path.canonicalize().map_err(|error| {
        format!(
            "failed to resolve {description} {}: {error}",
            path.display()
        )
    })?;
    if !canonical.starts_with(canonical_repo_root) {
        return Err(format!(
            "{description} {} escapes repository containment {}",
            path.display(),
            canonical_repo_root.display()
        ));
    }
    Ok(canonical)
}

fn parse_node_package(canonical_repo_root: &Path, path: &Path) -> Result<NodePackage, String> {
    let canonical = canonical_contained_path(canonical_repo_root, path, "Node package manifest")?;
    let content = fs::read_to_string(&canonical)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    serde_json::from_str::<NodePackage>(&content)
        .map_err(|error| format!("failed to parse {}: {error}", path.display()))
}

fn parse_manifest_value(
    canonical_repo_root: &Path,
    path: &Path,
) -> Result<serde_json::Value, String> {
    let canonical = canonical_contained_path(canonical_repo_root, path, "Node package manifest")?;
    let content = fs::read_to_string(&canonical)
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
    use super::{NodeWorkspace, collect};
    use ayni_core::{
        AyniPolicy, DepsPolicy, ExecutionResolution, Offenders, RunContext, Scope, SignalResult,
    };
    use std::collections::BTreeMap;

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
            cancellation: Default::default(),
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
            cancellation: Default::default(),
            debug: false,
        };

        let row = collect(&context).expect("dependency row");

        assert!(row.pass);
        assert_eq!(row.scope.package.as_deref(), Some("app"));
    }

    #[test]
    fn relative_repository_root_keeps_nested_workspace_edges_visible() {
        let current_dir = std::env::current_dir()
            .expect("current directory")
            .canonicalize()
            .expect("canonical current directory");
        let directory = tempfile::Builder::new()
            .prefix("ayni-node-deps-")
            .tempdir_in(&current_dir)
            .expect("relative fixture");
        let repo_root = directory
            .path()
            .strip_prefix(&current_dir)
            .expect("fixture below current directory")
            .to_path_buf();
        assert!(repo_root.is_relative());
        let target = repo_root.join("frontend");
        std::fs::create_dir_all(target.join("packages/base")).expect("base package");
        std::fs::create_dir_all(target.join("packages/app")).expect("app package");
        std::fs::write(
            target.join("package.json"),
            r#"{"name":"frontend","workspaces":["packages/*"]}"#,
        )
        .expect("workspace");
        std::fs::write(
            target.join("packages/base/package.json"),
            r#"{"name":"@example/base"}"#,
        )
        .expect("base manifest");
        std::fs::write(
            target.join("packages/app/package.json"),
            r#"{"name":"@example/app","dependencies":{"@example/base":"workspace:*"}}"#,
        )
        .expect("app manifest");

        let mut policy = AyniPolicy::default();
        policy.node.deps = Some(DepsPolicy {
            forbidden: BTreeMap::from([(
                String::from("frontend/packages/app"),
                vec![String::from("frontend/packages/base")],
            )]),
        });
        let context = RunContext {
            repo_root: repo_root.clone(),
            target_root: target.clone(),
            workdir: target.clone(),
            policy,
            scope: Scope {
                workspace_root: repo_root.to_string_lossy().into_owned(),
                path: Some(String::from("frontend")),
                package: None,
                file: None,
            },
            execution: ExecutionResolution::direct("pnpm", target, "lock", 100),
            cancellation: Default::default(),
            debug: false,
        };

        let row = collect(&context).expect("dependency row");
        let SignalResult::Deps(result) = &row.result else {
            panic!("deps result");
        };
        let Offenders::Deps(offenders) = &row.offenders else {
            panic!("deps offenders");
        };
        assert!(!row.pass);
        assert_eq!(result.crate_count, 3);
        assert_eq!(result.edge_count, 1);
        assert_eq!(result.violation_count, 1);
        assert_eq!(offenders.len(), 1);
        assert_eq!(offenders[0].from, "frontend/packages/app");
        assert_eq!(offenders[0].to, "frontend/packages/base");
        assert_eq!(
            offenders[0].rule,
            "frontend/packages/app -> frontend/packages/base"
        );
    }

    #[test]
    fn full_root_with_no_visible_members_fails_instead_of_passing() {
        let directory = tempfile::tempdir().expect("fixture");
        let workspace = NodeWorkspace {
            members: Vec::new(),
        };
        let error = workspace
            .visible_members(
                &Scope {
                    workspace_root: directory.path().to_string_lossy().into_owned(),
                    path: None,
                    package: None,
                    file: None,
                },
                directory.path(),
            )
            .expect_err("empty full-root evidence must fail");

        assert!(error.contains("resolved no visible workspace members"));
        assert!(error.contains("non-package scope '.'"));
    }

    #[test]
    fn file_scope_without_a_visible_member_fails_collection() {
        let directory = tempfile::tempdir().expect("fixture");
        let repo_root = directory
            .path()
            .canonicalize()
            .expect("canonical repository");
        let target = repo_root.join("frontend");
        std::fs::create_dir(&target).expect("frontend");
        std::fs::write(target.join("package.json"), r#"{"name":"frontend"}"#)
            .expect("frontend manifest");
        std::fs::write(repo_root.join("outside.ts"), "export {};\n").expect("outside file");
        let context = RunContext {
            repo_root: repo_root.clone(),
            target_root: target.clone(),
            workdir: target.clone(),
            policy: AyniPolicy::default(),
            scope: Scope {
                workspace_root: repo_root.to_string_lossy().into_owned(),
                path: Some(String::from("frontend")),
                package: None,
                file: Some(String::from("outside.ts")),
            },
            execution: ExecutionResolution::direct("npm", target, "test", 100),
            cancellation: Default::default(),
            debug: false,
        };

        let error = collect(&context).expect_err("unowned file scope must fail collection");
        assert!(error.contains("resolved no visible workspace members"));
        assert!(error.contains("non-package scope 'outside.ts'"));
    }

    #[cfg(unix)]
    #[test]
    fn workspace_member_symlink_escape_fails_containment() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().expect("fixture");
        let repo_root = directory.path().join("repository");
        let target = repo_root.join("frontend");
        let outside = directory.path().join("outside");
        std::fs::create_dir_all(target.join("packages")).expect("packages");
        std::fs::create_dir(&outside).expect("outside");
        std::fs::write(
            target.join("package.json"),
            r#"{"name":"frontend","workspaces":["packages/*"]}"#,
        )
        .expect("workspace");
        std::fs::write(outside.join("secret.txt"), "outside\n").expect("outside input");
        symlink(&outside, target.join("packages/escape")).expect("escaping member");

        let canonical_repo = repo_root.canonicalize().expect("canonical repository");
        let canonical_target = canonical_repo.join("frontend");
        let context = RunContext {
            repo_root: canonical_repo.clone(),
            target_root: canonical_target.clone(),
            workdir: canonical_target.clone(),
            policy: AyniPolicy::default(),
            scope: Scope {
                workspace_root: canonical_repo.to_string_lossy().into_owned(),
                path: Some(String::from("frontend")),
                package: None,
                file: None,
            },
            execution: ExecutionResolution::direct("npm", canonical_target, "test", 100),
            cancellation: Default::default(),
            debug: false,
        };

        let error = collect(&context).expect_err("escaping workspace member must fail");
        assert!(error.contains("Node workspace directory"));
        assert!(error.contains("escapes repository containment"));
    }

    #[cfg(unix)]
    #[test]
    fn package_manifest_symlink_escape_fails_containment() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().expect("fixture");
        let repo_root = directory.path().join("repository");
        let target = repo_root.join("frontend");
        let outside_manifest = directory.path().join("outside-package.json");
        std::fs::create_dir_all(&target).expect("frontend");
        std::fs::write(&outside_manifest, r#"{"name":"frontend"}"#).expect("outside manifest");
        symlink(&outside_manifest, target.join("package.json")).expect("escaping manifest");

        let canonical_repo = repo_root.canonicalize().expect("canonical repository");
        let canonical_target = canonical_repo.join("frontend");
        let context = RunContext {
            repo_root: canonical_repo.clone(),
            target_root: canonical_target.clone(),
            workdir: canonical_target.clone(),
            policy: AyniPolicy::default(),
            scope: Scope {
                workspace_root: canonical_repo.to_string_lossy().into_owned(),
                path: Some(String::from("frontend")),
                package: None,
                file: None,
            },
            execution: ExecutionResolution::direct("npm", canonical_target, "test", 100),
            cancellation: Default::default(),
            debug: false,
        };

        let error = collect(&context).expect_err("escaping manifest must fail");
        assert!(error.contains("Node package manifest"));
        assert!(error.contains("escapes repository containment"));
    }

    #[cfg(unix)]
    #[test]
    fn unrelated_dangling_symlink_does_not_block_discovery() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().expect("fixture");
        let repo_root = directory
            .path()
            .canonicalize()
            .expect("canonical repository");
        let target = repo_root.join("frontend");
        std::fs::create_dir(&target).expect("frontend");
        std::fs::write(target.join("package.json"), r#"{"name":"frontend"}"#).expect("workspace");
        symlink(target.join("missing"), target.join("unrelated-link")).expect("dangling link");

        let context = RunContext {
            repo_root: repo_root.clone(),
            target_root: target.clone(),
            workdir: target.clone(),
            policy: AyniPolicy::default(),
            scope: Scope {
                workspace_root: repo_root.to_string_lossy().into_owned(),
                path: Some(String::from("frontend")),
                package: None,
                file: None,
            },
            execution: ExecutionResolution::direct("npm", target, "test", 100),
            cancellation: Default::default(),
            debug: false,
        };

        let row = collect(&context).expect("irrelevant dangling link must be ignored");
        let SignalResult::Deps(result) = &row.result else {
            panic!("deps result");
        };
        assert_eq!(result.crate_count, 1);
    }

    #[cfg(unix)]
    #[test]
    fn workspace_directory_cycle_is_not_descended() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().expect("fixture");
        let repo_root = directory
            .path()
            .canonicalize()
            .expect("canonical repository");
        let target = repo_root.join("frontend");
        std::fs::create_dir_all(target.join("packages/base")).expect("base package");
        std::fs::write(
            target.join("package.json"),
            r#"{"name":"frontend","workspaces":["packages/*"]}"#,
        )
        .expect("workspace");
        std::fs::write(
            target.join("packages/base/package.json"),
            r#"{"name":"base"}"#,
        )
        .expect("base manifest");
        symlink(&target, target.join("packages/cycle")).expect("workspace cycle");

        let context = RunContext {
            repo_root: repo_root.clone(),
            target_root: target.clone(),
            workdir: target.clone(),
            policy: AyniPolicy::default(),
            scope: Scope {
                workspace_root: repo_root.to_string_lossy().into_owned(),
                path: Some(String::from("frontend")),
                package: None,
                file: None,
            },
            execution: ExecutionResolution::direct("npm", target, "test", 100),
            cancellation: Default::default(),
            debug: false,
        };

        let row = collect(&context).expect("cyclic alias must not prevent collection");
        let SignalResult::Deps(result) = &row.result else {
            panic!("deps result");
        };
        assert!(row.pass);
        assert_eq!(result.crate_count, 2);
        assert_eq!(result.edge_count, 0);
    }
}
