use std::fs;
use std::path::Path;

pub fn discover_roots(repo_root: &Path) -> Vec<String> {
    let mut roots = discover_file_parent_roots(repo_root, "package.json", |parts| {
        parts.contains(&"node_modules")
    });

    let root_package_json = repo_root.join("package.json");
    if root_package_json.is_file()
        && let Ok(content) = fs::read_to_string(&root_package_json)
        && let Ok(value) = serde_json::from_str::<serde_json::Value>(&content)
    {
        let mut patterns = Vec::new();
        if let Some(array) = value.get("workspaces").and_then(|v| v.as_array()) {
            patterns.extend(array.iter().filter_map(|item| item.as_str()));
        }
        if let Some(array) = value
            .get("workspaces")
            .and_then(|v| v.get("packages"))
            .and_then(|v| v.as_array())
        {
            patterns.extend(array.iter().filter_map(|item| item.as_str()));
        }
        for pattern in patterns {
            append_workspace_roots(repo_root, pattern, &mut roots);
        }
    }

    dedupe_and_sort_roots(roots)
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
