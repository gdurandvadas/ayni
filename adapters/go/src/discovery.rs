use ayni_adapters_common::discovery::{discover_file_parent_roots, is_vcs_or_vendor_dir};
use ayni_core::{DiscoveredRoot, ProjectDiscovery, ProjectLayout};
use std::path::Path;

pub fn discover_roots(repo_root: &Path) -> Vec<String> {
    discover_project_roots(repo_root).analyzable_roots()
}

pub fn discover_project_roots(repo_root: &Path) -> ProjectDiscovery {
    let roots = discover_file_parent_roots(repo_root, "go.mod", is_vcs_or_vendor_dir);
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
