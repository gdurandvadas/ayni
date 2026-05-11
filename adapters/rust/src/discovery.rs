use ayni_core::{DiscoveredRoot, ProjectDiscovery, ProjectLayout};
use std::fs;
use std::path::Path;

pub fn discover_roots(repo_root: &Path) -> Vec<String> {
    discover_project_roots(repo_root).analyzable_roots()
}

pub fn discover_project_roots(repo_root: &Path) -> ProjectDiscovery {
    let roots = discover_file_parent_roots(repo_root, "Cargo.toml", |parts| {
        parts.contains(&"target") || parts.contains(&".git") || parts.contains(&"node_modules")
    });
    let root_manifest = repo_root.join("Cargo.toml");
    let controlled = cargo_manifest_has_table(&root_manifest, "workspace");
    let root_analyzable = cargo_manifest_has_table(&root_manifest, "package");
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

fn cargo_manifest_has_table(path: &Path, table: &str) -> bool {
    let Ok(content) = fs::read_to_string(path) else {
        return false;
    };
    let Ok(value) = toml::from_str::<toml::Value>(&content) else {
        return false;
    };
    value.get(table).is_some()
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
    fn workspace_controller_only_root_excludes_workspace_root() {
        let dir = TempDir::new().expect("tempdir");
        fs::write(
            dir.path().join("Cargo.toml"),
            r#"[workspace]
members = ["crates/api"]
"#,
        )
        .expect("workspace manifest");
        fs::create_dir_all(dir.path().join("crates/api")).expect("api dir");
        fs::write(
            dir.path().join("crates/api/Cargo.toml"),
            r#"[package]
name = "api"
version = "0.1.0"
edition = "2021"
"#,
        )
        .expect("api manifest");

        let discovery = discover_project_roots(dir.path());

        assert_eq!(discovery.layout, ProjectLayout::ControlledMonorepo);
        assert_eq!(discover_roots(dir.path()), vec![String::from("crates/api")]);
    }

    #[test]
    fn workspace_with_package_includes_workspace_root() {
        let dir = TempDir::new().expect("tempdir");
        fs::write(
            dir.path().join("Cargo.toml"),
            r#"[package]
name = "root"
version = "0.1.0"
edition = "2021"

[workspace]
members = ["crates/api"]
"#,
        )
        .expect("workspace manifest");
        fs::create_dir_all(dir.path().join("crates/api")).expect("api dir");
        fs::write(
            dir.path().join("crates/api/Cargo.toml"),
            r#"[package]
name = "api"
version = "0.1.0"
edition = "2021"
"#,
        )
        .expect("api manifest");

        assert_eq!(
            discover_roots(dir.path()),
            vec![String::from("."), String::from("crates/api")]
        );
    }
}
