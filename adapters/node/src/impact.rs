use ayni_core::{
    AdapterError, ChangeKind, ImpactCapability, ImpactConfidence, ImpactContribution, ImpactReason,
    ImpactReasonKind, ImpactRequest, ImpactUncertainty, ImpactUncertaintyKind, Language,
    SelectedCheck, SignalKind,
};
use glob::{MatchOptions, Pattern};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use walkdir::{DirEntry, WalkDir};

pub struct NodeImpactCapability;

impl ImpactCapability for NodeImpactCapability {
    fn language(&self) -> Language {
        Language::Node
    }

    fn analyze(&self, request: &ImpactRequest) -> Result<ImpactContribution, AdapterError> {
        let topology_root = node_topology_root(request)
            .map_err(|error| AdapterError::new(Language::Node, error))?;
        let (packages, nonmember_dirs) = packages(&topology_root, request.repo_root())
            .map_err(|error| AdapterError::new(Language::Node, error))?;
        let affected = match affected_packages(request, &packages, &nonmember_dirs, &topology_root)
        {
            PackageImpact::Irrelevant => return Ok(empty_contribution(request)),
            PackageImpact::Broad(kind, detail) => {
                return Ok(broad_contribution(request, kind, detail));
            }
            PackageImpact::Affected(packages) => packages,
        };
        let mut result = empty_contribution(request);
        add_package_checks(request, &packages, &affected, &mut result);
        add_file_checks(request, &mut result);
        Ok(result)
    }
}

enum PackageImpact {
    Irrelevant,
    Affected(BTreeSet<String>),
    Broad(ImpactUncertaintyKind, String),
}

fn empty_contribution(request: &ImpactRequest) -> ImpactContribution {
    ImpactContribution {
        language: Language::Node,
        configured_root: request.configured_root.clone(),
        selected_checks: Vec::new(),
        uncertainties: Vec::new(),
    }
}

fn affected_packages(
    request: &ImpactRequest,
    packages: &BTreeMap<String, Package>,
    nonmember_dirs: &[String],
    topology_root: &std::path::Path,
) -> PackageImpact {
    let topology_prefix = repository_relative_dir(request.repo_root(), topology_root);
    let mut affected = BTreeSet::new();
    let mut relevant_source = false;
    for change in &request.changes {
        for path in change_paths(change) {
            if let Some(local) = path_in_target(path, &topology_prefix) {
                if is_manifest(&local) {
                    return PackageImpact::Broad(
                        ImpactUncertaintyKind::TopologyChanged,
                        format!("{path} changed npm workspace topology"),
                    );
                }
                if is_configuration_input(&local) {
                    return PackageImpact::Broad(
                        ImpactUncertaintyKind::ConfigurationChanged,
                        format!("{path} is a configuration-sensitive Node input"),
                    );
                }
            }
            let Some(local) = path_in_target(path, &request.configured_root) else {
                continue;
            };
            if is_source(&local) {
                relevant_source = true;
                if nonmember_dirs.iter().any(|dir| contains_path(dir, path)) {
                    return PackageImpact::Broad(
                        ImpactUncertaintyKind::UnknownPathOwnership,
                        format!(
                            "{path} is below a package outside declared Node workspace members"
                        ),
                    );
                }
                let owners = package_owners(packages, path);
                if owners.len() != 1 {
                    return PackageImpact::Broad(
                        ImpactUncertaintyKind::UnknownPathOwnership,
                        format!("cannot uniquely map {path} to an npm package"),
                    );
                }
                affected.insert(owners[0].clone());
            }
        }
    }
    if relevant_source {
        PackageImpact::Affected(affected)
    } else {
        PackageImpact::Irrelevant
    }
}

fn add_package_checks(
    request: &ImpactRequest,
    packages: &BTreeMap<String, Package>,
    affected: &BTreeSet<String>,
    result: &mut ImpactContribution,
) {
    for package in reverse_closure(packages, affected) {
        for signal in &request.enabled_signals {
            let reason = package_reason(&package, affected.contains(&package));
            match signal {
                SignalKind::Test | SignalKind::Deps => result.selected_checks.push(package_check(
                    request,
                    package.clone(),
                    *signal,
                    reason,
                )),
                SignalKind::Coverage | SignalKind::Mutation => broaden_one(
                    request,
                    result,
                    *signal,
                    "coverage and mutation require root-comparable execution",
                ),
                SignalKind::Size | SignalKind::Complexity => {}
            }
        }
    }
}

fn add_file_checks(request: &ImpactRequest, result: &mut ImpactContribution) {
    let mut broad = false;
    for change in &request.changes {
        let Some(local) = path_in_target(&change.path, &request.configured_root) else {
            continue;
        };
        if !is_source(&local) {
            continue;
        }
        if focusable_file_change(request, change) {
            add_focused_file_checks(request, result, &change.path);
        } else {
            broad = true;
        }
    }
    if broad {
        broaden_file_checks(request, result);
    }
}

fn focusable_file_change(request: &ImpactRequest, change: &ayni_core::ChangedPath) -> bool {
    matches!(change.kind, ChangeKind::Added | ChangeKind::Modified)
        && request.repo_root().join(&change.path).is_file()
}

fn add_focused_file_checks(request: &ImpactRequest, result: &mut ImpactContribution, path: &str) {
    for signal in [SignalKind::Size, SignalKind::Complexity] {
        if request.enabled_signals.contains(&signal) {
            result.selected_checks.push(SelectedCheck {
                language: Language::Node,
                configured_root: request.configured_root.clone(),
                package: None,
                file: Some(path.to_owned()),
                signal,
                reasons: vec![ImpactReason {
                    kind: ImpactReasonKind::ChangedFile,
                    detail: format!("{path} changed"),
                }],
                confidence: ImpactConfidence::High,
            });
        }
    }
}

fn broaden_file_checks(request: &ImpactRequest, result: &mut ImpactContribution) {
    for signal in [SignalKind::Size, SignalKind::Complexity] {
        if request.enabled_signals.contains(&signal) {
            broaden_one(
                request,
                result,
                signal,
                "deleted, renamed, copied, or type-changed Node files require root accounting",
            );
        }
    }
}

struct Package {
    dir: String,
    deps: BTreeSet<String>,
}

fn node_topology_root(request: &ImpactRequest) -> Result<std::path::PathBuf, String> {
    let target = request.configured_root_path();
    let mut current = target.as_path();
    loop {
        let manifest = current.join("package.json");
        if manifest.is_file() {
            let value: Value = serde_json::from_str(
                &std::fs::read_to_string(&manifest).map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())?;
            if value.get("workspaces").is_some() {
                return Ok(current.to_path_buf());
            }
        }
        if current == request.repo_root() {
            return Ok(target);
        }
        let Some(parent) = current
            .parent()
            .filter(|parent| parent.starts_with(request.repo_root()))
        else {
            return Ok(target);
        };
        current = parent;
    }
}

fn repository_relative_dir(repo_root: &std::path::Path, path: &std::path::Path) -> String {
    let relative = path
        .strip_prefix(repo_root)
        .expect("impact topology root is repository-contained")
        .to_string_lossy()
        .replace('\\', "/");
    if relative.is_empty() {
        String::from(".")
    } else {
        relative
    }
}

fn workspace_pattern_matches(pattern: &Pattern, path: &str) -> bool {
    pattern.matches_with(
        path,
        MatchOptions {
            case_sensitive: true,
            require_literal_separator: true,
            require_literal_leading_dot: false,
        },
    )
}

fn node_workspace_patterns(root: &std::path::Path) -> Result<Vec<Pattern>, String> {
    let value: Value = serde_json::from_str(
        &std::fs::read_to_string(root.join("package.json")).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    let patterns = match value.get("workspaces") {
        Some(Value::Array(patterns)) => Some(patterns),
        Some(Value::Object(workspaces)) => workspaces.get("packages").and_then(Value::as_array),
        _ => None,
    };
    patterns
        .into_iter()
        .flatten()
        .map(|value| {
            let pattern = value
                .as_str()
                .ok_or_else(|| String::from("Node workspace path pattern must be a string"))?;
            Pattern::new(pattern.trim_end_matches('/')).map_err(|error| error.to_string())
        })
        .collect()
}

fn packages(
    topology_root: &std::path::Path,
    repo_root: &std::path::Path,
) -> Result<(BTreeMap<String, Package>, Vec<String>), String> {
    let workspace_patterns = node_workspace_patterns(topology_root)?;
    let mut output = BTreeMap::new();
    let mut nonmember_dirs = Vec::new();
    for entry in WalkDir::new(topology_root)
        .into_iter()
        .filter_entry(relevant_entry)
    {
        let entry = entry.map_err(|error| format!("failed to traverse npm workspace: {error}"))?;
        if entry.file_name() != "package.json" {
            continue;
        }
        match classify_manifest(entry.path(), topology_root, repo_root, &workspace_patterns)? {
            ManifestClassification::Nonmember(dir) => nonmember_dirs.push(dir),
            ManifestClassification::Member(name, package) => {
                output.insert(name, package);
            }
        }
    }
    nonmember_dirs.sort();
    nonmember_dirs.dedup();
    Ok((output, nonmember_dirs))
}

enum ManifestClassification {
    Nonmember(String),
    Member(String, Package),
}

fn classify_manifest(
    path: &std::path::Path,
    topology_root: &std::path::Path,
    repo_root: &std::path::Path,
    workspace_patterns: &[Pattern],
) -> Result<ManifestClassification, String> {
    let parent = path.parent().expect("manifest parent");
    let topology_dir = repository_relative_dir(topology_root, parent);
    let repository_dir = repository_relative_dir(repo_root, parent);
    if topology_dir != "."
        && !workspace_patterns
            .iter()
            .any(|pattern| workspace_pattern_matches(pattern, &topology_dir))
    {
        return Ok(ManifestClassification::Nonmember(repository_dir));
    }
    let value: Value =
        serde_json::from_str(&std::fs::read_to_string(path).map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())?;
    let name = value
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or(&repository_dir)
        .to_owned();
    let mut deps = BTreeSet::new();
    for key in [
        "dependencies",
        "devDependencies",
        "peerDependencies",
        "optionalDependencies",
    ] {
        if let Some(map) = value.get(key).and_then(Value::as_object) {
            deps.extend(map.keys().cloned());
        }
    }
    Ok(ManifestClassification::Member(
        name,
        Package {
            dir: repository_dir,
            deps,
        },
    ))
}

fn relevant_entry(entry: &DirEntry) -> bool {
    !entry.file_type().is_dir()
        || !matches!(
            entry.file_name().to_str(),
            Some(".git" | ".ayni" | "node_modules" | "target" | "dist" | "build" | "coverage")
        )
}

fn package_owners(packages: &BTreeMap<String, Package>, path: &str) -> Vec<String> {
    let mut owners = packages
        .iter()
        .filter(|(_, package)| contains_path(&package.dir, path))
        .map(|(name, package)| (path_depth(&package.dir), name.clone()))
        .collect::<Vec<_>>();
    let deepest = owners.iter().map(|(depth, _)| *depth).max();
    owners.retain(|(depth, _)| Some(*depth) == deepest);
    owners.into_iter().map(|(_, name)| name).collect()
}

fn contains_path(dir: &str, path: &str) -> bool {
    dir == "." || path == dir || path.starts_with(&format!("{dir}/"))
}

fn path_depth(path: &str) -> usize {
    if path == "." {
        0
    } else {
        path.split('/').count()
    }
}

fn path_in_target(path: &str, configured_root: &str) -> Option<String> {
    if configured_root == "." {
        Some(path.to_owned())
    } else if path == configured_root {
        Some(String::from("."))
    } else {
        path.strip_prefix(&format!("{configured_root}/"))
            .map(str::to_owned)
    }
}

fn change_paths(change: &ayni_core::ChangedPath) -> impl Iterator<Item = &str> {
    std::iter::once(change.path.as_str()).chain(change.previous_path.as_deref())
}

fn is_manifest(path: &str) -> bool {
    [
        "package.json",
        "package-lock.json",
        "npm-shrinkwrap.json",
        "pnpm-lock.yaml",
        "yarn.lock",
        "bun.lock",
        "bun.lockb",
    ]
    .iter()
    .any(|name| path == *name || path.ends_with(&format!("/{name}")))
}

fn is_configuration_input(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    [".json", ".yaml", ".yml", ".toml"]
        .iter()
        .any(|extension| name.ends_with(extension))
        || name.contains(".config.")
        || name.starts_with("config.")
        || name == ".env"
        || name.ends_with("rc")
}

fn is_source(path: &str) -> bool {
    [".js", ".jsx", ".ts", ".tsx", ".mjs", ".cjs"]
        .iter()
        .any(|extension| path.ends_with(extension))
}

fn reverse_closure(
    packages: &BTreeMap<String, Package>,
    seeds: &BTreeSet<String>,
) -> BTreeSet<String> {
    let mut result = seeds.clone();
    loop {
        let before = result.len();
        for (name, package) in packages {
            if package
                .deps
                .iter()
                .any(|dependency| result.contains(dependency))
            {
                result.insert(name.clone());
            }
        }
        if before == result.len() {
            return result;
        }
    }
}

fn package_reason(package: &str, directly_affected: bool) -> ImpactReason {
    ImpactReason {
        kind: if directly_affected {
            ImpactReasonKind::PackageOwnership
        } else {
            ImpactReasonKind::ReverseDependency
        },
        detail: if directly_affected {
            format!("changed source belongs to {package}")
        } else {
            format!("{package} transitively reverse-depends on a changed package")
        },
    }
}

fn package_check(
    request: &ImpactRequest,
    package: String,
    signal: SignalKind,
    reason: ImpactReason,
) -> SelectedCheck {
    SelectedCheck {
        language: Language::Node,
        configured_root: request.configured_root.clone(),
        package: Some(package),
        file: None,
        signal,
        reasons: vec![reason],
        confidence: ImpactConfidence::High,
    }
}

fn broad_contribution(
    request: &ImpactRequest,
    kind: ImpactUncertaintyKind,
    detail: String,
) -> ImpactContribution {
    let mut result = ImpactContribution {
        language: Language::Node,
        configured_root: request.configured_root.clone(),
        selected_checks: Vec::new(),
        uncertainties: vec![ImpactUncertainty {
            kind,
            detail: detail.clone(),
        }],
    };
    for signal in &request.enabled_signals {
        broaden_one(request, &mut result, *signal, &detail);
    }
    result
}

fn broaden_one(
    request: &ImpactRequest,
    result: &mut ImpactContribution,
    signal: SignalKind,
    detail: &str,
) {
    result.selected_checks.push(SelectedCheck::root(
        Language::Node,
        request.configured_root.clone(),
        signal,
        ImpactReason {
            kind: ImpactReasonKind::ConservativeBroadening,
            detail: detail.to_owned(),
        },
        ImpactConfidence::Medium,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use ayni_core::{ChangeKind, ChangedPath};
    use tempfile::tempdir;

    #[test]
    fn selects_transitive_reverse_dependencies_and_prefers_deepest_owner() {
        let dir = tempdir().expect("repo");
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"name":"root","workspaces":["packages/*"]}"#,
        )
        .expect("root package");
        for (name, dependencies) in [
            ("base", "{}"),
            ("middle", r#"{"base":"workspace:*"}"#),
            ("app", r#"{"middle":"workspace:*"}"#),
        ] {
            let package = dir.path().join("packages").join(name);
            std::fs::create_dir_all(package.join("src")).expect("package");
            std::fs::write(
                package.join("package.json"),
                format!(r#"{{"name":"{name}","dependencies":{dependencies}}}"#),
            )
            .expect("manifest");
        }
        std::fs::write(
            dir.path().join("packages/base/src/index.ts"),
            "export const base = 1;\n",
        )
        .expect("source");
        std::fs::create_dir_all(dir.path().join("examples/shadow")).expect("example");
        std::fs::write(
            dir.path().join("examples/shadow/package.json"),
            r#"{"name":"base"}"#,
        )
        .expect("non-member manifest");
        let request = ImpactRequest::new(
            dir.path().canonicalize().expect("canonical"),
            Language::Node,
            String::from("packages/base"),
            vec![ChangedPath {
                kind: ChangeKind::Modified,
                path: String::from("packages/base/src/index.ts"),
                previous_path: None,
            }],
            [SignalKind::Test],
        )
        .expect("request");
        let contribution = ayni_adapters_common::impact::assert_impact_capability_conformance(
            &NodeImpactCapability,
            &request,
        )
        .expect("impact");
        let selected = contribution
            .selected_checks
            .iter()
            .filter_map(|check| check.package.as_deref())
            .collect::<BTreeSet<_>>();
        assert_eq!(selected, BTreeSet::from(["app", "base", "middle"]));
        assert!(contribution.uncertainties.is_empty());
    }

    #[test]
    fn nonmember_package_source_broadens_instead_of_using_root_ownership() {
        let dir = tempdir().expect("repo");
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"name":"root","workspaces":["packages/*"]}"#,
        )
        .expect("workspace");
        std::fs::create_dir_all(dir.path().join("examples/demo/src")).expect("example");
        std::fs::write(
            dir.path().join("examples/demo/package.json"),
            r#"{"name":"demo"}"#,
        )
        .expect("manifest");
        let request = ImpactRequest::new(
            dir.path().canonicalize().expect("canonical"),
            Language::Node,
            String::from("."),
            vec![ChangedPath {
                kind: ChangeKind::Modified,
                path: String::from("examples/demo/src/index.ts"),
                previous_path: None,
            }],
            [SignalKind::Test],
        )
        .expect("request");

        let contribution = NodeImpactCapability.analyze(&request).expect("impact");

        assert_eq!(contribution.selected_checks.len(), 1);
        assert_eq!(
            contribution.uncertainties[0].kind,
            ImpactUncertaintyKind::UnknownPathOwnership
        );
    }

    #[test]
    fn configuration_change_broadens_every_signal() {
        let dir = tempdir().expect("repo");
        std::fs::write(dir.path().join("package.json"), r#"{"name":"root"}"#).expect("manifest");
        let request = ImpactRequest::new(
            dir.path().canonicalize().expect("canonical"),
            Language::Node,
            String::from("."),
            vec![ChangedPath {
                kind: ChangeKind::Modified,
                path: String::from("vitest.config.ts"),
                previous_path: None,
            }],
            [SignalKind::Test, SignalKind::Complexity],
        )
        .expect("request");

        let contribution = ayni_adapters_common::impact::assert_impact_capability_conformance(
            &NodeImpactCapability,
            &request,
        )
        .expect("impact");

        assert_eq!(contribution.selected_checks.len(), 2);
        assert_eq!(
            contribution.uncertainties[0].kind,
            ImpactUncertaintyKind::ConfigurationChanged
        );
        assert!(
            contribution
                .selected_checks
                .iter()
                .all(|check| check.package.is_none() && check.file.is_none())
        );
    }
}
