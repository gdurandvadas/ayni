use crate::workspace::WorkspacePatterns;
use ayni_adapters_common::discovery::{dedupe_and_sort_roots, discover_file_parent_roots};
use ayni_adapters_common::paths::canonicalize_relative_posix;
use ayni_core::{DiscoveredRoot, ProjectDiscovery, ProjectLayout};
use std::fs;
use std::path::Path;

pub fn discover_roots(repo_root: &Path) -> Vec<String> {
    discover_project_roots(repo_root).analyzable_roots()
}

pub fn discover_project_roots(repo_root: &Path) -> ProjectDiscovery {
    let roots = dedupe_and_sort_roots(discover_file_parent_roots(
        repo_root,
        "package.json",
        |parts| parts.contains(&"node_modules"),
    ));

    let root_package_json = repo_root.join("package.json");
    let controlled = fs::read_to_string(&root_package_json)
        .ok()
        .and_then(|content| serde_json::from_str::<serde_json::Value>(&content).ok())
        .and_then(|manifest| WorkspacePatterns::parse(&manifest, &root_package_json).ok())
        .is_some_and(|patterns| !patterns.is_empty());
    let root_analyzable = if controlled {
        let package_roots = roots
            .iter()
            .filter(|root| root.as_str() != ".")
            .cloned()
            .collect::<Vec<_>>();
        root_has_source_files_outside_packages(repo_root, &package_roots)
    } else {
        root_package_json.is_file()
    };
    let layout = if controlled {
        ProjectLayout::ControlledMonorepo
    } else if roots.len() == 1 && roots.first().is_some_and(|root| root == ".") {
        ProjectLayout::SingleRoot
    } else {
        ProjectLayout::UncontrolledMonorepo
    };
    ProjectDiscovery {
        layout,
        roots: roots
            .into_iter()
            .map(|path| {
                let analyzable = path != "." || root_analyzable;
                DiscoveredRoot { path, analyzable }
            })
            .collect(),
    }
}

fn root_has_source_files_outside_packages(repo_root: &Path, package_roots: &[String]) -> bool {
    let mut stack = vec![repo_root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(value) => value,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if should_skip_dir(repo_root, &path, package_roots) {
                    continue;
                }
                stack.push(path);
                continue;
            }
            if is_node_source_file(&path) {
                return true;
            }
        }
    }
    false
}

fn should_skip_dir(repo_root: &Path, path: &Path, package_roots: &[String]) -> bool {
    let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
        return false;
    };
    if matches!(
        name,
        "node_modules" | ".git" | ".ayni" | "dist" | "build" | "coverage"
    ) {
        return true;
    }
    let Ok(relative) = path.strip_prefix(repo_root) else {
        return false;
    };
    let text = canonicalize_relative_posix(&relative.to_string_lossy());
    package_roots.contains(&text)
}

fn is_node_source_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|value| value.to_str()),
        Some("js" | "jsx" | "ts" | "tsx" | "mjs" | "cjs")
    )
}

#[cfg(test)]
mod tests {
    use super::{discover_project_roots, discover_roots};
    use ayni_core::ProjectLayout;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn workspace_controller_without_root_sources_excludes_workspace_root() {
        let dir = TempDir::new().expect("tempdir");
        fs::write(
            dir.path().join("package.json"),
            r#"{"workspaces":["packages/*"]}"#,
        )
        .expect("root package");
        fs::create_dir_all(dir.path().join("packages/api")).expect("api dir");
        fs::write(dir.path().join("packages/api/package.json"), "{}").expect("api package");

        let discovery = discover_project_roots(dir.path());

        assert_eq!(discovery.layout, ProjectLayout::ControlledMonorepo);
        assert_eq!(
            discover_roots(dir.path()),
            vec![String::from("packages/api")]
        );
    }

    #[test]
    fn general_and_negated_workspace_patterns_keep_nonmembers_standalone() {
        let dir = TempDir::new().expect("tempdir");
        fs::write(
            dir.path().join("package.json"),
            r#"{"workspaces":["packages/**","!packages/excluded/**"]}"#,
        )
        .expect("root package");
        for root in ["packages/api", "packages/excluded/app", "tools/standalone"] {
            fs::create_dir_all(dir.path().join(root)).expect("package dir");
            fs::write(dir.path().join(root).join("package.json"), "{}").expect("package manifest");
        }

        let discovery = discover_project_roots(dir.path());

        assert_eq!(discovery.layout, ProjectLayout::ControlledMonorepo);
        assert_eq!(
            discover_roots(dir.path()),
            vec![
                String::from("packages/api"),
                String::from("packages/excluded/app"),
                String::from("tools/standalone"),
            ]
        );
    }

    #[test]
    fn workspace_controller_with_root_sources_includes_workspace_root() {
        let dir = TempDir::new().expect("tempdir");
        fs::write(
            dir.path().join("package.json"),
            r#"{"workspaces":{"packages":["packages/*"]}}"#,
        )
        .expect("root package");
        fs::write(dir.path().join("index.ts"), "export {};\n").expect("source");
        fs::create_dir_all(dir.path().join("packages/api")).expect("api dir");
        fs::write(dir.path().join("packages/api/package.json"), "{}").expect("api package");

        assert_eq!(
            discover_roots(dir.path()),
            vec![String::from("."), String::from("packages/api")]
        );
    }

    #[test]
    fn uncontrolled_packages_only_repo_excludes_workspace_root() {
        let dir = TempDir::new().expect("tempdir");
        fs::create_dir_all(dir.path().join("packages/api")).expect("api dir");
        fs::write(dir.path().join("packages/api/package.json"), "{}").expect("api package");
        fs::create_dir_all(dir.path().join("packages/worker")).expect("worker dir");
        fs::write(dir.path().join("packages/worker/package.json"), "{}").expect("worker package");

        let discovery = discover_project_roots(dir.path());

        assert_eq!(discovery.layout, ProjectLayout::UncontrolledMonorepo);
        assert_eq!(
            discovery.policy_roots(),
            vec![
                String::from("packages/api"),
                String::from("packages/worker")
            ]
        );
    }
}
