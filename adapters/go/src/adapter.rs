use crate::catalog::GO_CATALOG;
use crate::collectors::GoCollector;
use crate::discovery;
use ayni_core::{
    CatalogEntry, DetectResult, ExecutionResolution, Language, LanguageAdapter, LanguageProfile,
    SignalCollector,
};
use std::path::Path;

#[derive(Debug, Default)]
pub struct GoAdapter {
    collector: GoCollector,
}

impl GoAdapter {
    #[must_use]
    pub fn new() -> Self {
        Self {
            collector: GoCollector,
        }
    }
}

impl LanguageAdapter for GoAdapter {
    fn language(&self) -> Language {
        Language::Go
    }

    fn detect(&self, root: &Path) -> DetectResult {
        let detected = root.join("go.mod").is_file();
        DetectResult {
            detected,
            confidence: if detected { 100 } else { 0 },
            reason: if detected {
                Some(format!("go.mod found at {}", root.display()))
            } else {
                Some(format!("go.mod not found at {}", root.display()))
            },
        }
    }

    fn resolve_execution(&self, repo_root: &Path, root: &Path) -> Option<ExecutionResolution> {
        if !root.join("go.mod").is_file() {
            return None;
        }
        if let Some(workspace) = find_go_work_ancestor(repo_root, root) {
            return Some(ExecutionResolution {
                runner: String::from("go"),
                resolved_from: workspace,
                kind: String::from("workspace_ancestor"),
                source: String::from("go.work"),
                confidence: 90,
                ambiguous: false,
                install_cwd: root.to_path_buf(),
                exec_cwd: root.to_path_buf(),
            });
        }
        Some(ExecutionResolution::direct(
            "go",
            root.to_path_buf(),
            "go.mod",
            100,
        ))
    }

    fn discover_roots(&self, repo_root: &Path) -> Vec<String> {
        discovery::discover_roots(repo_root)
    }

    fn profile(&self) -> LanguageProfile {
        LanguageProfile {
            language: Language::Go,
            default_file_globs: vec![String::from("*.go")],
        }
    }

    fn catalog(&self) -> &'static [CatalogEntry] {
        GO_CATALOG
    }

    fn collector(&self) -> &dyn SignalCollector {
        &self.collector
    }
}

fn find_go_work_ancestor(repo_root: &Path, root: &Path) -> Option<std::path::PathBuf> {
    let mut current = root.parent();
    while let Some(path) = current {
        if !path.starts_with(repo_root) {
            break;
        }
        if path.join("go.work").is_file() {
            return Some(path.to_path_buf());
        }
        current = path.parent();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::GoAdapter;
    use ayni_core::LanguageAdapter;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn resolves_go_work_ancestor_but_executes_module_root() {
        let dir = TempDir::new().expect("tempdir");
        fs::write(dir.path().join("go.work"), "go 1.22\nuse ./services/api\n").expect("go work");
        fs::create_dir_all(dir.path().join("services/api")).expect("api dir");
        fs::write(
            dir.path().join("services/api/go.mod"),
            "module example.com/api\n\ngo 1.22\n",
        )
        .expect("go mod");

        let adapter = GoAdapter::new();
        let module = dir.path().join("services/api");
        let resolution = adapter
            .resolve_execution(dir.path(), &module)
            .expect("resolution");

        assert_eq!(resolution.runner, "go");
        assert_eq!(resolution.kind, "workspace_ancestor");
        assert_eq!(resolution.resolved_from, dir.path());
        assert_eq!(resolution.exec_cwd, module);
    }
}
