use ayni_core::{DiscoveredRoot, ProjectDiscovery, ProjectLayout};
use std::fs;
use std::path::Path;

pub fn discover_roots(repo_root: &Path) -> Vec<String> {
    discover_project_roots(repo_root).analyzable_roots()
}

pub fn discover_project_roots(repo_root: &Path) -> ProjectDiscovery {
    let mut roots = discover_file_parent_roots(repo_root, "package.json", |parts| {
        parts.contains(&"node_modules")
    });

    let mut workspace_patterns = Vec::new();
    let root_package_json = repo_root.join("package.json");
    if root_package_json.is_file()
        && let Ok(content) = fs::read_to_string(&root_package_json)
        && let Ok(value) = serde_json::from_str::<serde_json::Value>(&content)
    {
        if let Some(array) = value.get("workspaces").and_then(|v| v.as_array()) {
            workspace_patterns.extend(
                array
                    .iter()
                    .filter_map(|item| item.as_str())
                    .map(String::from),
            );
        }
        if let Some(array) = value
            .get("workspaces")
            .and_then(|v| v.get("packages"))
            .and_then(|v| v.as_array())
        {
            workspace_patterns.extend(
                array
                    .iter()
                    .filter_map(|item| item.as_str())
                    .map(String::from),
            );
        }
        for pattern in &workspace_patterns {
            append_workspace_roots(repo_root, pattern, &mut roots);
        }
    }

    let roots = dedupe_and_sort_roots(roots);
    let controlled = !workspace_patterns.is_empty();
    let root_analyzable = if controlled {
        root_has_source_files_outside_workspace_members(repo_root, &workspace_patterns)
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

fn append_workspace_roots(repo_root: &Path, pattern: &str, roots: &mut Vec<String>) {
    if !pattern.ends_with("/*") {
        return;
    }
    let base = pattern.trim_end_matches("/*").trim_matches('/');
    if base.is_empty() {
        return;
    }
    let base_path = repo_root.join(base);
    if let Ok(entries) = fs::read_dir(base_path) {
        for entry in entries.flatten() {
            let candidate_dir = entry.path();
            if !candidate_dir.is_dir() {
                continue;
            }
            if candidate_dir.join("package.json").is_file()
                && let Ok(relative) = candidate_dir.strip_prefix(repo_root)
            {
                roots.push(canonicalize_relative_posix(&relative.to_string_lossy()));
            }
        }
    }
}

fn root_has_source_files_outside_workspace_members(repo_root: &Path, patterns: &[String]) -> bool {
    let workspace_bases = workspace_base_dirs(patterns);
    let mut stack = vec![repo_root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(value) => value,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if should_skip_dir(repo_root, &path, &workspace_bases) {
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

fn workspace_base_dirs(patterns: &[String]) -> Vec<String> {
    patterns
        .iter()
        .filter_map(|pattern| pattern.strip_suffix("/*"))
        .map(|base| canonicalize_relative_posix(base.trim_matches('/')))
        .filter(|base| base != ".")
        .collect()
}

fn should_skip_dir(repo_root: &Path, path: &Path, workspace_bases: &[String]) -> bool {
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
    workspace_bases
        .iter()
        .any(|base| text == *base || text.starts_with(&format!("{base}/")))
}

fn is_node_source_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|value| value.to_str()),
        Some("js" | "jsx" | "ts" | "tsx" | "mjs" | "cjs")
    )
}

fn discover_file_parent_roots<F>(repo_root: &Path, file_name: &str, exclude: F) -> Vec<String>
where
    F: Fn(&[&str]) -> bool,
{
    let mut found = Vec::new();
    let mut stack = vec![repo_root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(value) => value,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Ok(relative) = path.strip_prefix(repo_root) {
                    let text = canonicalize_relative_posix(&relative.to_string_lossy());
                    let parts: Vec<&str> = text.split('/').collect();
                    if exclude(&parts) {
                        continue;
                    }
                }
                stack.push(path);
                continue;
            }
            if path.file_name().and_then(|v| v.to_str()) != Some(file_name) {
                continue;
            }
            if let Some(parent) = path.parent()
                && let Ok(relative) = parent.strip_prefix(repo_root)
            {
                found.push(canonicalize_relative_posix(&relative.to_string_lossy()));
            }
        }
    }
    dedupe_and_sort_roots(found)
}

fn dedupe_and_sort_roots(mut roots: Vec<String>) -> Vec<String> {
    roots.sort();
    roots.dedup();
    roots
}

fn canonicalize_relative_posix(value: &str) -> String {
    let mut normalized = value.trim().replace('\\', "/");
    while normalized.ends_with('/') {
        normalized.pop();
    }
    if normalized.is_empty() {
        String::from(".")
    } else {
        normalized
    }
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
