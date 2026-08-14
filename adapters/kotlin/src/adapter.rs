use crate::catalog::KOTLIN_CATALOG;
use crate::collectors::KotlinCollector;
use crate::discovery;
use ayni_adapters_common::catalog::GENERIC_CATALOG_RUNTIME;
use ayni_adapters_common::finding::{DependencySource, target_for_finding};
use ayni_core::{
    CatalogEntry, CatalogOperation, CatalogOperationError, CatalogOperationErrorKind,
    CatalogRuntime, ComplexityThresholdKind, DetectResult, ExecutionResolution, Language,
    LanguageAdapter, LanguageProfile, OffenderIdentity, PolicyEffectivenessFacts, ProjectDiscovery,
    Scope, SignalCollector, SignalKind, ToolStatus, VerificationSelectorSupport,
    VerificationTarget,
};
use std::path::Path;
use std::time::Duration;

struct KotlinCatalogRuntime;

static KOTLIN_CATALOG_RUNTIME: KotlinCatalogRuntime = KotlinCatalogRuntime;

impl CatalogRuntime for KotlinCatalogRuntime {
    fn status(
        &self,
        entry: &CatalogEntry,
        execution: &ExecutionResolution,
        timeout: Duration,
    ) -> Result<ToolStatus, CatalogOperationError> {
        GENERIC_CATALOG_RUNTIME.status(entry, execution, timeout)
    }

    fn prepare(
        &self,
        execution: &ExecutionResolution,
        _timeout: Duration,
        _on_line: &mut dyn FnMut(&str),
    ) -> Result<(), CatalogOperationError> {
        crate::install::ensure_gradle_plugins(&execution.install_cwd).map_err(|message| {
            CatalogOperationError::new(
                CatalogOperation::Prepare,
                CatalogOperationErrorKind::Contract,
                None,
                Some(execution.install_cwd.clone()),
                None,
                message,
            )
        })
    }

    fn install(
        &self,
        entry: &CatalogEntry,
        execution: &ExecutionResolution,
        timeout: Duration,
        on_line: &mut dyn FnMut(&str),
    ) -> Result<(), CatalogOperationError> {
        GENERIC_CATALOG_RUNTIME.install(entry, execution, timeout, on_line)
    }
}

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
            environment: std::collections::BTreeMap::new(),
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

    fn catalog_runtime(&self) -> &dyn CatalogRuntime {
        &KOTLIN_CATALOG_RUNTIME
    }

    fn collector(&self) -> &dyn SignalCollector {
        &self.collector
    }

    fn policy_effectiveness_facts(&self) -> PolicyEffectivenessFacts {
        PolicyEffectivenessFacts::new(
            Language::Kotlin,
            vec![ComplexityThresholdKind::FnCyclomatic],
        )
    }

    fn verification_selector_support(&self, kind: SignalKind) -> VerificationSelectorSupport {
        match kind {
            SignalKind::Test => VerificationSelectorSupport::new(false, true, true),
            SignalKind::Size | SignalKind::Complexity => {
                VerificationSelectorSupport::new(true, false, false)
            }
            SignalKind::Coverage | SignalKind::Deps | SignalKind::Mutation => {
                VerificationSelectorSupport::NONE
            }
        }
    }

    fn verification_target(
        &self,
        kind: SignalKind,
        scope: &Scope,
        offender: OffenderIdentity<'_>,
    ) -> VerificationTarget {
        target_for_finding(
            kind,
            self.verification_selector_support(kind),
            scope,
            offender,
            DependencySource::Unscoped,
        )
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
    use ayni_core::{
        LanguageAdapter, OffenderIdentity, Scope, SignalKind, TestFailure,
        VerificationSelectorSupport,
    };
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

    #[test]
    fn declares_honest_selector_support() {
        let adapter = KotlinAdapter::new();
        assert_eq!(
            adapter.verification_selector_support(SignalKind::Test),
            VerificationSelectorSupport::new(false, true, true)
        );
        for kind in [SignalKind::Size, SignalKind::Complexity] {
            assert_eq!(
                adapter.verification_selector_support(kind),
                VerificationSelectorSupport::new(true, false, false)
            );
        }
        for kind in [SignalKind::Coverage, SignalKind::Deps, SignalKind::Mutation] {
            assert_eq!(
                adapter.verification_selector_support(kind),
                VerificationSelectorSupport::NONE
            );
        }
    }

    #[test]
    fn finding_zero_test_uses_supported_package_scope() {
        let adapter = KotlinAdapter::new();
        let offender = TestFailure {
            file: None,
            line: None,
            message: String::from("discovered zero tests"),
            test_name: None,
        };
        let scope = Scope {
            package: Some(String::from("com.example.ApiTest")),
            ..Scope::default()
        };
        let target = adapter.verification_target(
            SignalKind::Test,
            &scope,
            OffenderIdentity::Test(&offender),
        );
        adapter
            .verification_selector_support(SignalKind::Test)
            .validate_target(SignalKind::Test, &target)
            .expect("mapped target must match declared support");
        assert_eq!(target.package, scope.package);
    }
}
