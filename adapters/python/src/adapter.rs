use crate::catalog::PYTHON_CATALOG;
use crate::collectors::PythonCollector;
use ayni_core::{
    CatalogEntry, DetectResult, Language, LanguageAdapter, LanguageProfile, SignalCollector,
    detect_python_package_manager,
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
