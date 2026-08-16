use ayni_core::{
    AdapterError, ChangeKind, ImpactCapability, ImpactConfidence, ImpactContribution, ImpactReason,
    ImpactReasonKind, ImpactRequest, ImpactUncertainty, ImpactUncertaintyKind, Language,
    SelectedCheck, SignalKind,
};
use glob::{MatchOptions, Pattern};
use std::collections::{BTreeMap, BTreeSet};
use walkdir::{DirEntry, WalkDir};

pub struct RustImpactCapability;

impl ImpactCapability for RustImpactCapability {
    fn language(&self) -> Language {
        Language::Rust
    }

    fn analyze(&self, request: &ImpactRequest) -> Result<ImpactContribution, AdapterError> {
        let topology_root = cargo_topology_root(request)
            .map_err(|error| AdapterError::new(Language::Rust, error))?;
        let (packages, nonmember_dirs) = packages(&topology_root, request.repo_root())
            .map_err(|error| AdapterError::new(Language::Rust, error))?;
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
        language: Language::Rust,
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
    for change in &request.changes {
        for path in change_paths(change) {
            match classify_path(request, packages, nonmember_dirs, &topology_prefix, path) {
                PathImpact::Irrelevant => {}
                PathImpact::Affected(package) => {
                    affected.insert(package);
                }
                PathImpact::Broad(kind, detail) => return PackageImpact::Broad(kind, detail),
            }
        }
    }
    if affected.is_empty() {
        PackageImpact::Irrelevant
    } else {
        PackageImpact::Affected(affected)
    }
}

enum PathImpact {
    Irrelevant,
    Affected(String),
    Broad(ImpactUncertaintyKind, String),
}

fn classify_path(
    request: &ImpactRequest,
    packages: &BTreeMap<String, Package>,
    nonmember_dirs: &[String],
    topology_prefix: &str,
    path: &str,
) -> PathImpact {
    if is_governing_rust_input(path, &request.configured_root) {
        return PathImpact::Broad(
            ImpactUncertaintyKind::ConfigurationChanged,
            format!("{path} is a governing Rust runtime or Cargo input"),
        );
    }
    if let Some(local) = path_in_target(path, topology_prefix) {
        if is_manifest(&local) {
            return PathImpact::Broad(
                ImpactUncertaintyKind::TopologyChanged,
                format!("{path} changed Cargo topology"),
            );
        }
        if is_configuration_input(&local) {
            return PathImpact::Broad(
                ImpactUncertaintyKind::ConfigurationChanged,
                format!("{path} is a configuration-sensitive Rust input"),
            );
        }
    }
    let Some(local) = path_in_target(path, &request.configured_root) else {
        return PathImpact::Irrelevant;
    };
    if !local.ends_with(".rs") {
        return PathImpact::Irrelevant;
    }
    if nonmember_dirs.iter().any(|dir| contains_path(dir, path)) {
        return PathImpact::Broad(
            ImpactUncertaintyKind::UnknownPathOwnership,
            format!("{path} is below a Cargo manifest outside declared workspace members"),
        );
    }
    let owners = package_owners(packages, path);
    if owners.len() == 1 {
        PathImpact::Affected(owners[0].clone())
    } else {
        PathImpact::Broad(
            ImpactUncertaintyKind::UnknownPathOwnership,
            format!("cannot uniquely map {path} to a Cargo package"),
        )
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
        if !local.ends_with(".rs") {
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
                language: Language::Rust,
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
                "deleted, renamed, copied, or type-changed Rust files require root accounting",
            );
        }
    }
}

struct Package {
    dir: String,
    deps: BTreeSet<String>,
}

fn cargo_topology_root(request: &ImpactRequest) -> Result<std::path::PathBuf, String> {
    let target = request.configured_root_path();
    let mut current = target.as_path();
    loop {
        let manifest = current.join("Cargo.toml");
        if manifest.is_file() {
            let value: toml::Value = toml::from_str(
                &std::fs::read_to_string(&manifest).map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())?;
            if value.get("workspace").is_some() {
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

struct CargoWorkspaceMembership {
    members: Vec<Pattern>,
    excludes: Vec<Pattern>,
}

impl CargoWorkspaceMembership {
    fn load(root: &std::path::Path) -> Result<Self, String> {
        let value: toml::Value = toml::from_str(
            &std::fs::read_to_string(root.join("Cargo.toml")).map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
        let workspace = value.get("workspace");
        Ok(Self {
            members: compile_cargo_patterns(workspace.and_then(|value| value.get("members")))?,
            excludes: compile_cargo_patterns(workspace.and_then(|value| value.get("exclude")))?,
        })
    }

    fn includes(&self, dir: &str) -> bool {
        dir == "."
            || (!self
                .excludes
                .iter()
                .any(|pattern| workspace_pattern_matches(pattern, dir))
                && self
                    .members
                    .iter()
                    .any(|pattern| workspace_pattern_matches(pattern, dir)))
    }

    fn excludes(&self, dir: &str) -> bool {
        self.excludes
            .iter()
            .any(|pattern| workspace_pattern_matches(pattern, dir))
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

fn compile_cargo_patterns(value: Option<&toml::Value>) -> Result<Vec<Pattern>, String> {
    value
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .map(|value| {
            let pattern = value
                .as_str()
                .ok_or_else(|| String::from("Cargo workspace path pattern must be a string"))?;
            Pattern::new(pattern.trim_end_matches('/')).map_err(|error| error.to_string())
        })
        .collect()
}

fn packages(
    topology_root: &std::path::Path,
    repo_root: &std::path::Path,
) -> Result<(BTreeMap<String, Package>, Vec<String>), String> {
    let membership = CargoWorkspaceMembership::load(topology_root)?;
    let aliases = workspace_dependency_aliases(topology_root)?;
    let mut output = BTreeMap::new();
    let mut nonmember_dirs = Vec::new();
    for entry in WalkDir::new(topology_root)
        .into_iter()
        .filter_entry(relevant_entry)
    {
        let entry =
            entry.map_err(|error| format!("failed to traverse Cargo workspace: {error}"))?;
        if entry.file_name() != "Cargo.toml" {
            continue;
        }
        match classify_manifest(
            entry.path(),
            topology_root,
            repo_root,
            &membership,
            &aliases,
        )? {
            ManifestClassification::Virtual => {}
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
    Virtual,
    Nonmember(String),
    Member(String, Package),
}

fn classify_manifest(
    path: &std::path::Path,
    topology_root: &std::path::Path,
    repo_root: &std::path::Path,
    membership: &CargoWorkspaceMembership,
    aliases: &BTreeMap<String, String>,
) -> Result<ManifestClassification, String> {
    let value: toml::Value =
        toml::from_str(&std::fs::read_to_string(path).map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())?;
    let parent = path.parent().expect("manifest parent");
    let topology_dir = repository_relative_dir(topology_root, parent);
    let repository_dir = repository_relative_dir(repo_root, parent);
    if membership.excludes(&topology_dir) {
        return Ok(ManifestClassification::Nonmember(repository_dir));
    }
    let Some(name) = value
        .get("package")
        .and_then(|package| package.get("name"))
        .and_then(toml::Value::as_str)
    else {
        return Ok(ManifestClassification::Virtual);
    };
    if !membership.includes(&topology_dir) {
        return Ok(ManifestClassification::Nonmember(repository_dir));
    }
    Ok(ManifestClassification::Member(
        name.to_owned(),
        Package {
            dir: repository_dir,
            deps: package_dependencies(&value, aliases)?,
        },
    ))
}

fn package_dependencies(
    value: &toml::Value,
    aliases: &BTreeMap<String, String>,
) -> Result<BTreeSet<String>, String> {
    let mut deps = BTreeSet::new();
    collect_dependency_tables(value, aliases, &mut deps)?;
    if let Some(targets) = value.get("target").and_then(toml::Value::as_table) {
        for target in targets.values() {
            collect_dependency_tables(target, aliases, &mut deps)?;
        }
    }
    Ok(deps)
}

fn workspace_dependency_aliases(
    root: &std::path::Path,
) -> Result<BTreeMap<String, String>, String> {
    let manifest = root.join("Cargo.toml");
    if !manifest.is_file() {
        return Ok(BTreeMap::new());
    }
    let value: toml::Value =
        toml::from_str(&std::fs::read_to_string(manifest).map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())?;
    Ok(value
        .get("workspace")
        .and_then(|workspace| workspace.get("dependencies"))
        .and_then(toml::Value::as_table)
        .into_iter()
        .flat_map(|table| table.iter())
        .map(|(alias, dependency)| {
            let package = dependency
                .as_table()
                .and_then(|table| table.get("package"))
                .and_then(toml::Value::as_str)
                .unwrap_or(alias);
            (alias.clone(), package.to_owned())
        })
        .collect())
}

fn collect_dependency_tables(
    value: &toml::Value,
    aliases: &BTreeMap<String, String>,
    deps: &mut BTreeSet<String>,
) -> Result<(), String> {
    for key in ["dependencies", "dev-dependencies", "build-dependencies"] {
        let Some(table) = value.get(key).and_then(toml::Value::as_table) else {
            continue;
        };
        for (name, value) in table {
            let dependency = if value
                .as_table()
                .and_then(|table| table.get("workspace"))
                .and_then(toml::Value::as_bool)
                == Some(true)
            {
                aliases
                    .get(name)
                    .ok_or_else(|| format!("workspace dependency alias {name:?} is not declared"))?
            } else {
                value
                    .as_table()
                    .and_then(|table| table.get("package"))
                    .and_then(toml::Value::as_str)
                    .unwrap_or(name)
            };
            deps.insert(dependency.to_owned());
        }
    }
    Ok(())
}

fn relevant_entry(entry: &DirEntry) -> bool {
    !entry.file_type().is_dir()
        || !matches!(
            entry.file_name().to_str(),
            Some(".git" | ".ayni" | "target" | "node_modules")
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
    ["Cargo.toml", "Cargo.lock"]
        .iter()
        .any(|name| path == *name || path.ends_with(&format!("/{name}")))
}

fn is_governing_rust_input(path: &str, configured_root: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    let governing_dir = if path.ends_with(".cargo/config") {
        path.strip_suffix(".cargo/config")
            .unwrap_or("")
            .trim_end_matches('/')
    } else if path.ends_with(".cargo/config.toml") {
        path.strip_suffix(".cargo/config.toml")
            .unwrap_or("")
            .trim_end_matches('/')
    } else if matches!(name, "rust-toolchain" | "rust-toolchain.toml") {
        path.rsplit_once('/').map_or(".", |(parent, _)| parent)
    } else {
        return false;
    };
    governing_dir.is_empty()
        || governing_dir == "."
        || configured_root == governing_dir
        || configured_root.starts_with(&format!("{governing_dir}/"))
}

fn is_configuration_input(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    path == ".cargo/config"
        || path.ends_with("/.cargo/config")
        || name.starts_with("rust-toolchain")
        || [".toml", ".json", ".yaml", ".yml"]
            .iter()
            .any(|extension| name.ends_with(extension))
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
        language: Language::Rust,
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
        language: Language::Rust,
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
        Language::Rust,
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
    fn includes_transitive_reverse_dependents_and_prefers_deepest_owner() {
        let dir = tempdir().expect("repo");
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname='root'\nversion='0.1.0'\n[workspace]\nmembers=['a','b','c']\n[workspace.dependencies]\nalias={package='a',path='a'}\n",
        )
        .expect("root manifest");
        std::fs::create_dir(dir.path().join("src")).expect("root src");
        for (name, deps) in [
            ("a", ""),
            (
                "b",
                "[target.'cfg(unix)'.dependencies]\nalias={workspace=true}",
            ),
            ("c", "b={path='../b'}"),
        ] {
            std::fs::create_dir(dir.path().join(name)).expect("package");
            std::fs::create_dir(dir.path().join(name).join("src")).expect("src");
            std::fs::write(
                dir.path().join(name).join("Cargo.toml"),
                format!("[package]\nname='{name}'\nversion='0.1.0'\n[dependencies]\n{deps}"),
            )
            .expect("manifest");
        }
        std::fs::write(dir.path().join("a/src/lib.rs"), "pub fn a() {}\n").expect("source");
        let request = ImpactRequest::new(
            dir.path().canonicalize().expect("canonical"),
            Language::Rust,
            String::from("a"),
            vec![ChangedPath {
                kind: ChangeKind::Modified,
                path: String::from("a/src/lib.rs"),
                previous_path: None,
            }],
            [SignalKind::Test],
        )
        .expect("request");
        let contribution = ayni_adapters_common::impact::assert_impact_capability_conformance(
            &RustImpactCapability,
            &request,
        )
        .expect("impact");
        let selected = contribution
            .selected_checks
            .iter()
            .filter_map(|check| check.package.as_deref())
            .collect::<BTreeSet<_>>();
        assert_eq!(selected, BTreeSet::from(["a", "b", "c"]));
        assert!(contribution.uncertainties.is_empty());
    }

    #[test]
    fn undeclared_descendant_source_broadens_instead_of_claiming_workspace_ownership() {
        let dir = tempdir().expect("repo");
        std::fs::write(dir.path().join("Cargo.toml"), "[workspace]\nmembers=[]\n")
            .expect("workspace");
        std::fs::create_dir_all(dir.path().join("examples/demo/src")).expect("example");
        std::fs::write(
            dir.path().join("examples/demo/Cargo.toml"),
            "[package]\nname='demo'\nversion='0.1.0'\n",
        )
        .expect("manifest");
        let request = ImpactRequest::new(
            dir.path().canonicalize().expect("canonical"),
            Language::Rust,
            String::from("."),
            vec![ChangedPath {
                kind: ChangeKind::Modified,
                path: String::from("examples/demo/src/lib.rs"),
                previous_path: None,
            }],
            [SignalKind::Test],
        )
        .expect("request");

        let contribution = RustImpactCapability.analyze(&request).expect("impact");

        assert_eq!(contribution.selected_checks.len(), 1);
        assert_eq!(
            contribution.uncertainties[0].kind,
            ImpactUncertaintyKind::UnknownPathOwnership
        );
    }

    #[test]
    fn configuration_change_broadens_every_signal() {
        let dir = tempdir().expect("repo");
        std::fs::write(dir.path().join("Cargo.toml"), "[workspace]\nmembers=[]\n")
            .expect("manifest");
        let request = ImpactRequest::new(
            dir.path().canonicalize().expect("canonical"),
            Language::Rust,
            String::from("."),
            vec![ChangedPath {
                kind: ChangeKind::Modified,
                path: String::from(".cargo/config.toml"),
                previous_path: None,
            }],
            [SignalKind::Test, SignalKind::Complexity],
        )
        .expect("request");

        let contribution = ayni_adapters_common::impact::assert_impact_capability_conformance(
            &RustImpactCapability,
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

    #[test]
    fn ancestor_runtime_change_broadens_a_nested_workspace() {
        let dir = tempdir().expect("repo");
        std::fs::create_dir(dir.path().join("projects")).expect("workspace");
        std::fs::write(
            dir.path().join("projects/Cargo.toml"),
            "[workspace]\nmembers=[]\n",
        )
        .expect("manifest");
        let request = ImpactRequest::new(
            dir.path().canonicalize().expect("canonical"),
            Language::Rust,
            String::from("projects"),
            vec![ChangedPath {
                kind: ChangeKind::Modified,
                path: String::from("rust-toolchain.toml"),
                previous_path: None,
            }],
            [SignalKind::Test, SignalKind::Size],
        )
        .expect("request");

        let contribution = ayni_adapters_common::impact::assert_impact_capability_conformance(
            &RustImpactCapability,
            &request,
        )
        .expect("impact");

        assert_eq!(contribution.selected_checks.len(), 2);
        assert_eq!(
            contribution.uncertainties[0].kind,
            ImpactUncertaintyKind::ConfigurationChanged
        );
    }

    #[test]
    fn manifest_change_broadens_every_signal() {
        let dir = tempdir().expect("repo");
        std::fs::write(dir.path().join("Cargo.toml"), "[workspace]\nmembers=[]\n")
            .expect("manifest");
        let request = ImpactRequest::new(
            dir.path().canonicalize().expect("canonical"),
            Language::Rust,
            String::from("."),
            vec![ChangedPath {
                kind: ChangeKind::Modified,
                path: String::from("Cargo.toml"),
                previous_path: None,
            }],
            [SignalKind::Test, SignalKind::Size],
        )
        .expect("request");
        let contribution = ayni_adapters_common::impact::assert_impact_capability_conformance(
            &RustImpactCapability,
            &request,
        )
        .expect("impact");
        assert_eq!(contribution.selected_checks.len(), 2);
        assert!(
            contribution
                .selected_checks
                .iter()
                .all(|check| { check.package.is_none() && check.file.is_none() })
        );
    }
}
