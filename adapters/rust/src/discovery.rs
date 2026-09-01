use ayni_adapters_common::discovery::discover_file_parent_roots;
use ayni_core::{DiscoveredRoot, ProjectDiscovery, ProjectLayout};
use std::fs;
use std::path::Path;

pub fn discover_roots(repo_root: &Path) -> Vec<String> {
    discover_project_roots(repo_root).analyzable_roots()
}

pub fn discover_project_roots(repo_root: &Path) -> ProjectDiscovery {
    let roots = discover_file_parent_roots(repo_root, "Cargo.toml", |parts| {
        parts.contains(&"target") || parts.contains(&"node_modules")
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

#[cfg(test)]
mod tests {
    use super::{discover_project_roots, discover_roots};
    use ayni_core::ProjectLayout;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn excludes_manifests_in_universal_state_directories() {
        let dir = TempDir::new().expect("tempdir");
        fs::create_dir_all(dir.path().join("crates/live")).expect("live directory");
        fs::write(
            dir.path().join("crates/live/Cargo.toml"),
            "[package]\nname = \"live\"\nversion = \"0.1.0\"\n",
        )
        .expect("live manifest");
        for state in [".ayni", ".git"] {
            fs::create_dir_all(dir.path().join(state).join("nested")).expect("state directory");
            fs::write(
                dir.path().join(state).join("nested/Cargo.toml"),
                "[package]\nname = \"hidden\"\nversion = \"0.1.0\"\n",
            )
            .expect("state manifest");
        }

        assert_eq!(
            discover_roots(dir.path()),
            vec![String::from("crates/live")]
        );
    }

    #[cfg(unix)]
    #[test]
    fn skips_symlinked_directories_that_escape_the_repository() {
        use std::os::unix::fs::symlink;

        let repo = TempDir::new().expect("repository");
        let outside = TempDir::new().expect("outside");
        fs::write(
            outside.path().join("Cargo.toml"),
            "[package]\nname = \"escaped\"\nversion = \"0.1.0\"\n",
        )
        .expect("outside manifest");
        symlink(outside.path(), repo.path().join("escape")).expect("directory symlink");

        assert!(discover_roots(repo.path()).is_empty());
    }

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
