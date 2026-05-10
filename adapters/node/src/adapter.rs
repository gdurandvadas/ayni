use crate::catalog::NODE_CATALOG;
use crate::collectors::NodeCollector;
use crate::discovery;
use ayni_core::{
    CatalogEntry, DetectResult, ExecutionResolution, Language, LanguageAdapter, LanguageProfile,
    NodePackageManager, SignalCollector, detect_node_package_manager,
};
use std::path::{Path, PathBuf};

#[derive(Debug, Default)]
pub struct NodeAdapter {
    collector: NodeCollector,
}

impl NodeAdapter {
    #[must_use]
    pub fn new() -> Self {
        Self {
            collector: NodeCollector,
        }
    }
}

impl LanguageAdapter for NodeAdapter {
    fn language(&self) -> Language {
        Language::Node
    }

    fn detect(&self, root: &Path) -> DetectResult {
        let manifest = root.join("package.json");
        if !manifest.is_file() {
            return DetectResult {
                detected: false,
                confidence: 0,
                reason: Some(format!("package.json not found at {}", root.display())),
            };
        }

        let pm = detect_node_package_manager(root);
        let confidence = if pm.is_some() { 100 } else { 60 };
        let reason = if let Some(pm) = pm {
            format!(
                "package.json found at {}; package manager resolved as {}",
                root.display(),
                pm.executable()
            )
        } else {
            format!(
                "package.json found at {}; no lockfile/packageManager field (default runtime fallback)",
                root.display()
            )
        };

        DetectResult {
            detected: true,
            confidence,
            reason: Some(reason),
        }
    }

    fn resolve_execution(&self, repo_root: &Path, root: &Path) -> Option<ExecutionResolution> {
        let direct = detect_node_package_manager(root).map(|manager| {
            node_resolution(manager, root.to_path_buf(), "direct_root", false, 100, root)
        });
        let ancestor = find_node_workspace_ancestor(repo_root, root);
        match (direct, ancestor) {
            (Some(mut direct), Some(ancestor)) if direct.runner != ancestor.runner => {
                direct.ambiguous = true;
                Some(direct)
            }
            (Some(direct), _) => Some(direct),
            (None, Some(ancestor)) => Some(ancestor),
            (None, None) if root.join("package.json").is_file() => Some(node_resolution(
                NodePackageManager::Npm,
                root.to_path_buf(),
                "fallback",
                false,
                60,
                root,
            )),
            (None, None) => None,
        }
    }

    fn discover_roots(&self, repo_root: &Path) -> Vec<String> {
        discovery::discover_roots(repo_root)
    }

    fn profile(&self) -> LanguageProfile {
        LanguageProfile {
            language: Language::Node,
            default_file_globs: vec![
                String::from("*.js"),
                String::from("*.jsx"),
                String::from("*.ts"),
                String::from("*.tsx"),
                String::from("*.mjs"),
                String::from("*.cjs"),
            ],
        }
    }

    fn catalog(&self) -> &'static [CatalogEntry] {
        NODE_CATALOG
    }

    fn collector(&self) -> &dyn SignalCollector {
        &self.collector
    }
}

fn node_resolution(
    manager: NodePackageManager,
    resolved_from: PathBuf,
    kind: &str,
    ambiguous: bool,
    confidence: u8,
    exec_root: &Path,
) -> ExecutionResolution {
    ExecutionResolution {
        runner: manager.executable().to_string(),
        resolved_from: resolved_from.clone(),
        kind: kind.to_string(),
        source: String::from("node package manager"),
        confidence,
        ambiguous,
        install_cwd: resolved_from,
        exec_cwd: exec_root.to_path_buf(),
    }
}

fn find_node_workspace_ancestor(repo_root: &Path, root: &Path) -> Option<ExecutionResolution> {
    let mut current = root.parent();
    while let Some(path) = current {
        if !path.starts_with(repo_root) {
            break;
        }
        if path.join("package.json").is_file()
            && package_json_has_workspaces(&path.join("package.json"))
            && let Some(manager) = detect_node_package_manager(path)
        {
            return Some(node_resolution(
                manager,
                path.to_path_buf(),
                "workspace_ancestor",
                false,
                90,
                root,
            ));
        }
        current = path.parent();
    }
    None
}

fn package_json_has_workspaces(path: &Path) -> bool {
    let Ok(content) = std::fs::read_to_string(path) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&content) else {
        return false;
    };
    value.get("workspaces").is_some()
}

#[cfg(test)]
mod tests {
    use super::NodeAdapter;
    use ayni_core::LanguageAdapter;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn resolves_workspace_ancestor_package_manager() {
        let dir = TempDir::new().expect("tempdir");
        fs::write(
            dir.path().join("package.json"),
            r#"{"workspaces":["apps/*"],"packageManager":"pnpm@9.0.0"}"#,
        )
        .expect("root package");
        fs::write(dir.path().join("pnpm-lock.yaml"), "").expect("lockfile");
        fs::create_dir_all(dir.path().join("apps/api")).expect("api dir");
        fs::write(dir.path().join("apps/api/package.json"), "{}").expect("api package");

        let adapter = NodeAdapter::new();
        let resolution = adapter
            .resolve_execution(dir.path(), &dir.path().join("apps/api"))
            .expect("resolution");

        assert_eq!(resolution.runner, "pnpm");
        assert_eq!(resolution.kind, "workspace_ancestor");
        assert_eq!(resolution.install_cwd, dir.path());
        assert_eq!(resolution.exec_cwd, dir.path().join("apps/api"));
    }
}
