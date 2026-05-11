use ayni_core::{ProjectDiscovery, ProjectLayout};
use std::path::Path;

pub fn discover_roots(repo_root: &Path) -> Vec<String> {
    discover_project_roots(repo_root).analyzable_roots()
}

pub fn discover_project_roots(repo_root: &Path) -> ProjectDiscovery {
    if repo_root.join("build.gradle.kts").is_file()
        || repo_root.join("build.gradle").is_file()
        || repo_root.join("settings.gradle.kts").is_file()
        || repo_root.join("settings.gradle").is_file()
    {
        ProjectDiscovery::from_analyzable_roots(vec![String::from(".")])
    } else {
        ProjectDiscovery {
            layout: ProjectLayout::UncontrolledMonorepo,
            roots: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::discover_roots;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn only_discovers_configured_style_root() {
        let dir = TempDir::new().expect("tempdir");
        fs::write(
            dir.path().join("settings.gradle.kts"),
            "include(\":app\")\n",
        )
        .expect("settings");
        fs::create_dir_all(dir.path().join("app")).expect("app dir");
        fs::write(dir.path().join("app/build.gradle.kts"), "plugins {}\n").expect("app build");

        assert_eq!(discover_roots(dir.path()), vec![String::from(".")]);
    }
}
