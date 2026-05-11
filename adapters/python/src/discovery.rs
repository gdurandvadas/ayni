use ayni_core::{DiscoveredRoot, ProjectDiscovery, ProjectLayout};
use std::fs;
use std::path::Path;
use toml::Value;

const EXCLUDED_ROOTS: &[&str] = &[
    ".venv",
    "venv",
    "env",
    "__pycache__",
    ".tox",
    ".nox",
    ".git",
    ".ayni",
];

pub fn discover_roots(repo_root: &Path) -> Vec<String> {
    discover_project_roots(repo_root).analyzable_roots()
}

pub fn discover_project_roots(repo_root: &Path) -> ProjectDiscovery {
    let mut roots = Vec::new();
    for marker in ["pyproject.toml", "requirements.txt", "Pipfile"] {
        roots.extend(discover_file_parent_roots(repo_root, marker, |parts| {
            parts.iter().any(|part| EXCLUDED_ROOTS.contains(part))
        }));
    }
    let excluded = uv_workspace_excludes(repo_root);
    let roots = dedupe_and_sort_roots(roots)
        .into_iter()
        .filter(|root| !is_excluded_root(root, &excluded))
        .collect::<Vec<_>>();
    let controlled = is_uv_workspace_root(repo_root)
        || (repo_root.join("uv.lock").is_file() && roots.iter().any(|root| root != "."));
    let root_analyzable = is_root_python_project(repo_root);
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

fn is_root_python_project(repo_root: &Path) -> bool {
    repo_root.join("requirements.txt").is_file()
        || repo_root.join("Pipfile").is_file()
        || root_pyproject_has_project(repo_root)
}

fn root_pyproject_has_project(repo_root: &Path) -> bool {
    let pyproject_path = repo_root.join("pyproject.toml");
    let Ok(content) = fs::read_to_string(pyproject_path) else {
        return false;
    };
    let Ok(value) = toml::from_str::<Value>(&content) else {
        return false;
    };
    value.get("project").is_some()
}

fn is_uv_workspace_root(repo_root: &Path) -> bool {
    let pyproject_path = repo_root.join("pyproject.toml");
    let Ok(content) = fs::read_to_string(pyproject_path) else {
        return false;
    };
    let Ok(value) = toml::from_str::<Value>(&content) else {
        return false;
    };
    value
        .get("tool")
        .and_then(|value| value.get("uv"))
        .and_then(|value| value.get("workspace"))
        .is_some()
}

fn uv_workspace_excludes(repo_root: &Path) -> Vec<String> {
    let pyproject_path = repo_root.join("pyproject.toml");
    let Ok(content) = fs::read_to_string(pyproject_path) else {
        return Vec::new();
    };
    let Ok(value) = toml::from_str::<Value>(&content) else {
        return Vec::new();
    };
    value
        .get("tool")
        .and_then(|value| value.get("uv"))
        .and_then(|value| value.get("workspace"))
        .and_then(|value| value.get("exclude"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(canonicalize_relative_posix)
                .collect()
        })
        .unwrap_or_default()
}

fn is_excluded_root(root: &str, excluded: &[String]) -> bool {
    excluded
        .iter()
        .any(|pattern| glob::Pattern::new(pattern).is_ok_and(|glob| glob.matches(root)))
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
    fn excludes_environment_dirs() {
        let dir = TempDir::new().expect("tempdir");
        fs::write(
            dir.path().join("pyproject.toml"),
            "[project]\nname='root'\n",
        )
        .expect("root pyproject");
        fs::create_dir_all(dir.path().join("packages/api")).expect("api dir");
        fs::write(dir.path().join("packages/api/pyproject.toml"), "").expect("api pyproject");
        fs::create_dir_all(dir.path().join(".venv/lib")).expect("venv dir");
        fs::write(dir.path().join(".venv/lib/pyproject.toml"), "").expect("venv pyproject");

        assert_eq!(
            discover_roots(dir.path()),
            vec![String::from("."), String::from("packages/api")]
        );
    }

    #[test]
    fn uv_workspace_controller_only_root_excludes_workspace_root() {
        let dir = TempDir::new().expect("tempdir");
        fs::write(
            dir.path().join("pyproject.toml"),
            r#"
[tool.uv.workspace]
members = ["services/*"]
exclude = ["services/agent-runtime"]
"#,
        )
        .expect("root pyproject");
        fs::create_dir_all(dir.path().join("services/api")).expect("api dir");
        fs::write(dir.path().join("services/api/pyproject.toml"), "").expect("api pyproject");
        fs::create_dir_all(dir.path().join("services/agent-runtime")).expect("runtime dir");
        fs::write(dir.path().join("services/agent-runtime/pyproject.toml"), "")
            .expect("runtime pyproject");

        assert_eq!(
            discover_roots(dir.path()),
            vec![String::from("services/api")]
        );
        assert_eq!(
            discover_project_roots(dir.path()).layout,
            ProjectLayout::ControlledMonorepo
        );
    }

    #[test]
    fn uv_workspace_with_project_includes_workspace_root() {
        let dir = TempDir::new().expect("tempdir");
        fs::write(
            dir.path().join("pyproject.toml"),
            r#"
[project]
name = "root"

[tool.uv.workspace]
members = ["services/*"]
"#,
        )
        .expect("root pyproject");
        fs::create_dir_all(dir.path().join("services/api")).expect("api dir");
        fs::write(
            dir.path().join("services/api/pyproject.toml"),
            "[project]\nname='api'\n",
        )
        .expect("api pyproject");

        assert_eq!(
            discover_roots(dir.path()),
            vec![String::from("."), String::from("services/api")]
        );
    }

    #[test]
    fn excludes_uv_workspace_glob_patterns() {
        let dir = TempDir::new().expect("tempdir");
        fs::write(
            dir.path().join("pyproject.toml"),
            r#"
[tool.uv.workspace]
exclude = ["services/private-*"]
"#,
        )
        .expect("root pyproject");
        fs::create_dir_all(dir.path().join("services/private-api")).expect("private dir");
        fs::write(dir.path().join("services/private-api/pyproject.toml"), "")
            .expect("private pyproject");
        fs::create_dir_all(dir.path().join("services/public-api")).expect("public dir");
        fs::write(dir.path().join("services/public-api/pyproject.toml"), "")
            .expect("public pyproject");

        assert_eq!(
            discover_roots(dir.path()),
            vec![String::from("services/public-api")]
        );
    }

    #[test]
    fn single_root_project_uses_workspace_root() {
        let dir = TempDir::new().expect("tempdir");
        fs::write(
            dir.path().join("pyproject.toml"),
            "[project]\nname='root'\n",
        )
        .expect("root pyproject");

        let discovery = discover_project_roots(dir.path());

        assert_eq!(discovery.layout, ProjectLayout::SingleRoot);
        assert_eq!(discovery.policy_roots(), vec![String::from(".")]);
    }

    #[test]
    fn uncontrolled_packages_only_repo_excludes_workspace_root() {
        let dir = TempDir::new().expect("tempdir");
        fs::create_dir_all(dir.path().join("packages/api")).expect("api dir");
        fs::write(
            dir.path().join("packages/api/pyproject.toml"),
            "[project]\nname='api'\n",
        )
        .expect("api pyproject");
        fs::create_dir_all(dir.path().join("packages/worker")).expect("worker dir");
        fs::write(
            dir.path().join("packages/worker/pyproject.toml"),
            "[project]\nname='worker'\n",
        )
        .expect("worker pyproject");

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
