use ayni_core::{DiscoveredRoot, ProjectDiscovery, ProjectLayout};
use std::fs;
use std::path::Path;

pub fn discover_roots(repo_root: &Path) -> Vec<String> {
    discover_project_roots(repo_root).analyzable_roots()
}

pub fn discover_project_roots(repo_root: &Path) -> ProjectDiscovery {
    let roots = discover_file_parent_roots(repo_root, "go.mod");
    let controlled = repo_root.join("go.work").is_file();
    let root_analyzable = repo_root.join("go.mod").is_file();
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

fn discover_file_parent_roots(repo_root: &Path, file_name: &str) -> Vec<String> {
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
    found.sort();
    found.dedup();
    found
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
    fn go_work_controller_only_root_excludes_workspace_root() {
        let dir = TempDir::new().expect("tempdir");
        fs::write(dir.path().join("go.work"), "go 1.22\nuse ./services/api\n").expect("go work");
        fs::create_dir_all(dir.path().join("services/api")).expect("api dir");
        fs::write(
            dir.path().join("services/api/go.mod"),
            "module example.com/api\n\ngo 1.22\n",
        )
        .expect("go mod");

        let discovery = discover_project_roots(dir.path());

        assert_eq!(discovery.layout, ProjectLayout::ControlledMonorepo);
        assert_eq!(
            discover_roots(dir.path()),
            vec![String::from("services/api")]
        );
    }

    #[test]
    fn go_work_with_root_module_includes_workspace_root() {
        let dir = TempDir::new().expect("tempdir");
        fs::write(
            dir.path().join("go.work"),
            "go 1.22\nuse .\nuse ./services/api\n",
        )
        .expect("go work");
        fs::write(
            dir.path().join("go.mod"),
            "module example.com/root\n\ngo 1.22\n",
        )
        .expect("root go mod");
        fs::create_dir_all(dir.path().join("services/api")).expect("api dir");
        fs::write(
            dir.path().join("services/api/go.mod"),
            "module example.com/api\n\ngo 1.22\n",
        )
        .expect("go mod");

        assert_eq!(
            discover_roots(dir.path()),
            vec![String::from("."), String::from("services/api")]
        );
    }
}
