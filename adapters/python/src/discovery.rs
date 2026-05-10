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
    let mut roots = Vec::new();
    for marker in ["pyproject.toml", "requirements.txt", "Pipfile"] {
        roots.extend(discover_file_parent_roots(repo_root, marker, |parts| {
            parts.iter().any(|part| EXCLUDED_ROOTS.contains(part))
        }));
    }
    let excluded = uv_workspace_excludes(repo_root);
    dedupe_and_sort_roots(roots)
        .into_iter()
        .filter(|root| !is_excluded_root(root, &excluded))
        .collect()
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
    use super::discover_roots;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn excludes_environment_dirs() {
        let dir = TempDir::new().expect("tempdir");
        fs::write(dir.path().join("pyproject.toml"), "").expect("root pyproject");
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
    fn excludes_uv_workspace_excluded_roots() {
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
            vec![String::from("."), String::from("services/public-api")]
        );
    }
}
