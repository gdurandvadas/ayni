use crate::catalog::KOTLIN_CATALOG;
use crate::collectors::KotlinCollector;
use crate::discovery;
use ayni_core::{
    CatalogEntry, DetectResult, ExecutionResolution, Language, LanguageAdapter, LanguageProfile,
    ProjectDiscovery, SignalCollector,
};
use std::path::Path;

#[derive(Debug, Default)]
pub struct KotlinAdapter {
    collector: KotlinCollector,
}

impl KotlinAdapter {
    #[must_use]
    pub fn new() -> Self {
        Self {
            collector: KotlinCollector,
        }
    }
}

impl LanguageAdapter for KotlinAdapter {
    fn language(&self) -> Language {
        Language::Kotlin
    }

    fn detect(&self, root: &Path) -> DetectResult {
        let detected = has_gradle_marker(root);
        DetectResult {
            detected,
            confidence: if detected { 100 } else { 0 },
            reason: Some(if detected {
                format!("Gradle Kotlin root found at {}", root.display())
            } else {
                format!("Gradle Kotlin markers not found at {}", root.display())
            }),
        }
    }

    fn resolve_execution(&self, _repo_root: &Path, root: &Path) -> Option<ExecutionResolution> {
        if !has_gradle_marker(root) {
            return None;
        }
        let runner = gradle_runner(root);
        Some(ExecutionResolution {
            runner,
            resolved_from: root.to_path_buf(),
            kind: String::from("direct_root"),
            source: String::from("gradle build"),
            confidence: 100,
            ambiguous: false,
            install_cwd: root.to_path_buf(),
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
            language: Language::Kotlin,
            default_file_globs: vec![String::from("*.kt"), String::from("*.kts")],
        }
    }

    fn catalog(&self) -> &'static [CatalogEntry] {
        KOTLIN_CATALOG
    }

    fn collector(&self) -> &dyn SignalCollector {
        &self.collector
    }
}

fn has_gradle_marker(root: &Path) -> bool {
    root.join("build.gradle.kts").is_file()
        || root.join("build.gradle").is_file()
        || root.join("settings.gradle.kts").is_file()
        || root.join("settings.gradle").is_file()
}

fn gradle_runner(root: &Path) -> String {
    if root.join("gradlew").is_file() {
        String::from("./gradlew")
    } else if root.join("gradlew.bat").is_file() {
        String::from("gradlew.bat")
    } else {
        String::from("gradle")
    }
}

#[cfg(test)]
mod tests {
    use super::KotlinAdapter;
    use ayni_core::LanguageAdapter;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn resolves_gradle_wrapper_first() {
        let dir = TempDir::new().expect("tempdir");
        fs::write(dir.path().join("build.gradle.kts"), "plugins {}\n").expect("build");
        fs::write(dir.path().join("gradlew"), "").expect("wrapper");

        let adapter = KotlinAdapter::new();
        let resolution = adapter
            .resolve_execution(dir.path(), dir.path())
            .expect("resolution");

        assert_eq!(resolution.runner, "./gradlew");
        assert_eq!(resolution.kind, "direct_root");
        assert_eq!(resolution.install_cwd, dir.path());
    }
}
