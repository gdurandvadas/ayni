use crate::catalog::GO_CATALOG;
use crate::collectors::GoCollector;
use crate::discovery;
use crate::environment::GoEnvironmentCapability;
use crate::environment_resolution::GoEnvironmentResolutionCapability;
use crate::impact::GoImpactCapability;
use crate::preparation::GoDependencyPreparationCapability;
use ayni_adapters_common::catalog::GENERIC_CATALOG_RUNTIME;
use ayni_adapters_common::finding::{DependencySource, target_for_finding};
use ayni_core::{
    CatalogEntry, CatalogRuntime, ComplexityThresholdKind, DetectResult, ExecutionResolution,
    Language, LanguageAdapter, LanguageProfile, OffenderIdentity, PolicyEffectivenessFacts,
    ProjectDiscovery, Scope, SignalCollector, SignalKind, VerificationSelectorSupport,
    VerificationTarget,
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
                environment: std::collections::BTreeMap::new(),
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

    fn discover_project_roots(&self, repo_root: &Path) -> ProjectDiscovery {
        discovery::discover_project_roots(repo_root)
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

    fn catalog_runtime(&self) -> &dyn CatalogRuntime {
        &GENERIC_CATALOG_RUNTIME
    }

    fn impact_capability(&self) -> Option<&dyn ayni_core::ImpactCapability> {
        Some(&GoImpactCapability)
    }

    fn collector(&self) -> &dyn SignalCollector {
        &self.collector
    }

    fn environment_capability(&self) -> Option<&dyn ayni_core::EnvironmentCapability> {
        Some(&GoEnvironmentCapability)
    }

    fn dependency_preparation_capability(
        &self,
    ) -> Option<&dyn ayni_core::DependencyPreparationCapability> {
        Some(&GoDependencyPreparationCapability)
    }

    fn environment_resolution_capability(
        &self,
    ) -> Option<&dyn ayni_core::EnvironmentResolutionCapability> {
        Some(&GoEnvironmentResolutionCapability)
    }

    fn policy_effectiveness_facts(&self) -> PolicyEffectivenessFacts {
        PolicyEffectivenessFacts::new(Language::Go, vec![ComplexityThresholdKind::FnCyclomatic])
    }

    fn verification_selector_support(&self, kind: SignalKind) -> VerificationSelectorSupport {
        match kind {
            SignalKind::Test => VerificationSelectorSupport::new(false, true, true),
            SignalKind::Deps => VerificationSelectorSupport::new(true, true, false),
            SignalKind::Size | SignalKind::Complexity => {
                VerificationSelectorSupport::new(true, false, false)
            }
            SignalKind::Coverage | SignalKind::Mutation => VerificationSelectorSupport::NONE,
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
            DependencySource::Package,
        )
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
    use ayni_core::{
        LanguageAdapter, OffenderIdentity, Scope, SignalKind, TestFailure,
        VerificationSelectorSupport,
    };
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

    #[test]
    fn exposes_all_environment_capabilities() {
        let adapter = GoAdapter::new();
        assert!(adapter.environment_capability().is_some());
        assert!(adapter.dependency_preparation_capability().is_some());
        assert!(adapter.environment_resolution_capability().is_some());
    }

    #[test]
    fn declares_honest_selector_support() {
        let adapter = GoAdapter::new();
        assert_eq!(
            adapter.verification_selector_support(SignalKind::Test),
            VerificationSelectorSupport::new(false, true, true)
        );
        assert_eq!(
            adapter.verification_selector_support(SignalKind::Deps),
            VerificationSelectorSupport::new(true, true, false)
        );
        for kind in [SignalKind::Size, SignalKind::Complexity] {
            assert_eq!(
                adapter.verification_selector_support(kind),
                VerificationSelectorSupport::new(true, false, false)
            );
        }
        for kind in [SignalKind::Coverage, SignalKind::Mutation] {
            assert_eq!(
                adapter.verification_selector_support(kind),
                VerificationSelectorSupport::NONE
            );
        }
    }

    #[test]
    fn finding_test_name_maps_without_inventing_file_support() {
        let adapter = GoAdapter::new();
        let offender = TestFailure {
            file: Some(String::from("api_test.go")),
            line: Some(10),
            message: String::from("failed"),
            test_name: Some(String::from("TestCreate")),
        };
        let target = adapter.verification_target(
            SignalKind::Test,
            &Scope::default(),
            OffenderIdentity::Test(&offender),
        );
        adapter
            .verification_selector_support(SignalKind::Test)
            .validate_target(SignalKind::Test, &target)
            .expect("mapped target must match declared support");
        assert_eq!(target.name.as_deref(), Some("TestCreate"));
        assert!(target.file.is_none());
    }
}
