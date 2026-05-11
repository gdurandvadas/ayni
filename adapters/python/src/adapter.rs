use crate::catalog::PYTHON_CATALOG;
use crate::collectors::PythonCollector;
use crate::discovery;
use ayni_core::{
    CatalogEntry, DetectResult, ExecutionResolution, Language, LanguageAdapter, LanguageProfile,
    ProjectDiscovery, SignalCollector, detect_python_package_manager,
    resolve_python_package_manager,
};
use std::path::Path;

#[derive(Debug, Default)]
pub struct PythonAdapter {
    collector: PythonCollector,
}

impl PythonAdapter {
    #[must_use]
    pub fn new() -> Self {
        Self {
            collector: PythonCollector,
        }
    }
}

impl LanguageAdapter for PythonAdapter {
    fn language(&self) -> Language {
        Language::Python
    }

    fn detect(&self, root: &Path) -> DetectResult {
        let has_manifest = root.join("pyproject.toml").is_file()
            || root.join("requirements.txt").is_file()
            || root.join("Pipfile").is_file();
        if !has_manifest {
            return DetectResult {
                detected: false,
                confidence: 0,
                reason: Some(format!(
                    "pyproject.toml, requirements.txt, or Pipfile not found at {}",
                    root.display()
                )),
            };
        }

        let pm = detect_python_package_manager(root);
        let confidence = if pm.is_some() { 100 } else { 60 };
        let reason = if let Some(pm) = pm {
            format!(
                "python project found at {}; package manager resolved as {}",
                root.display(),
                pm.executable()
            )
        } else {
            format!(
                "python project found at {}; no lockfile/manager marker (default runtime fallback)",
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
        let resolution = resolve_python_package_manager(repo_root, root)?;
        let kind = resolution.kind_label().to_string();
        let runner = resolution.manager_label().to_string();
        let resolved_from = resolution.resolved_from;
        let install_cwd = match kind.as_str() {
            "workspace_ancestor" => resolved_from.clone(),
            _ => root.to_path_buf(),
        };
        Some(ExecutionResolution {
            runner,
            resolved_from,
            kind,
            source: String::from("python package manager"),
            confidence: if resolution.ambiguous { 80 } else { 100 },
            ambiguous: resolution.ambiguous,
            install_cwd,
            exec_cwd: root.to_path_buf(),
        })
    }

    fn discover_roots(&self, repo_root: &Path) -> Vec<String> {
        discovery::discover_roots(repo_root)
    }

    fn discover_project_roots(&self, repo_root: &Path) -> ProjectDiscovery {
        discovery::discover_project_roots(repo_root)
    }

    fn profile(&self) -> LanguageProfile {
        LanguageProfile {
            language: Language::Python,
            default_file_globs: vec![String::from("*.py")],
        }
    }

    fn catalog(&self) -> &'static [CatalogEntry] {
        PYTHON_CATALOG
    }

    fn collector(&self) -> &dyn SignalCollector {
        &self.collector
    }
}

#[cfg(test)]
mod tests {
    use super::PythonAdapter;
    use ayni_core::LanguageAdapter;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn resolves_uv_workspace_ancestor() {
        let dir = TempDir::new().expect("tempdir");
        fs::write(
            dir.path().join("pyproject.toml"),
            r#"[tool.uv.workspace]
members = ["libs/*"]
"#,
        )
        .expect("root pyproject");
        fs::write(dir.path().join("uv.lock"), "").expect("uv lock");
        fs::create_dir_all(dir.path().join("libs/math")).expect("math dir");
        fs::write(dir.path().join("libs/math/pyproject.toml"), "").expect("math pyproject");

        let adapter = PythonAdapter::new();
        let resolution = adapter
            .resolve_execution(dir.path(), &dir.path().join("libs/math"))
            .expect("resolution");

        assert_eq!(resolution.runner, "uv");
        assert_eq!(resolution.kind, "workspace_ancestor");
        assert_eq!(resolution.install_cwd, dir.path());
        assert_eq!(resolution.exec_cwd, dir.path().join("libs/math"));
    }
}
