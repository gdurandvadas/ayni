use crate::catalog::{PYTHON_CATALOG, PYTHON_CATALOG_RUNTIME};
use crate::collectors::PythonCollector;
use crate::discovery;
use crate::environment::PythonEnvironmentCapability;
use crate::environment_resolution::PythonEnvironmentResolutionCapability;
use crate::impact::PythonImpactCapability;
use crate::package_manager;
use crate::preparation::PythonDependencyPreparationCapability;
use ayni_adapters_common::finding::{DependencySource, target_for_finding};
use ayni_core::{
    CatalogEntry, CatalogRuntime, ComplexityThresholdKind, DetectResult, ExecutionResolution,
    Language, LanguageAdapter, LanguageProfile, OffenderIdentity, PolicyEffectivenessFacts,
    ProjectDiscovery, Scope, SignalCollector, SignalKind, VerificationSelectorSupport,
    VerificationTarget,
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

        let pm = package_manager::detect(root);
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
        package_manager::resolve(repo_root, root)
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

    fn catalog_runtime(&self) -> &dyn CatalogRuntime {
        &PYTHON_CATALOG_RUNTIME
    }

    fn environment_capability(&self) -> Option<&dyn ayni_core::EnvironmentCapability> {
        static CAPABILITY: PythonEnvironmentCapability = PythonEnvironmentCapability;
        Some(&CAPABILITY)
    }

    fn dependency_preparation_capability(
        &self,
    ) -> Option<&dyn ayni_core::DependencyPreparationCapability> {
        static CAPABILITY: PythonDependencyPreparationCapability =
            PythonDependencyPreparationCapability;
        Some(&CAPABILITY)
    }

    fn environment_resolution_capability(
        &self,
    ) -> Option<&dyn ayni_core::EnvironmentResolutionCapability> {
        static CAPABILITY: PythonEnvironmentResolutionCapability =
            PythonEnvironmentResolutionCapability;
        Some(&CAPABILITY)
    }

    fn impact_capability(&self) -> Option<&dyn ayni_core::ImpactCapability> {
        Some(&PythonImpactCapability)
    }

    fn collector(&self) -> &dyn SignalCollector {
        &self.collector
    }

    fn policy_effectiveness_facts(&self) -> PolicyEffectivenessFacts {
        PolicyEffectivenessFacts::new(Language::Python, vec![ComplexityThresholdKind::FnCognitive])
    }

    fn verification_selector_support(&self, kind: SignalKind) -> VerificationSelectorSupport {
        match kind {
            SignalKind::Test => VerificationSelectorSupport::new(true, true, true),
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
            DependencySource::File,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::PythonAdapter;
    use ayni_core::{
        LanguageAdapter, OffenderIdentity, Scope, SignalKind, SizeOffender,
        VerificationSelectorSupport,
    };
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn package_manager_resolution_matrix() {
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
        assert!(resolution.ambiguous);

        for (marker, manager) in [
            ("uv.lock", "uv"),
            ("poetry.lock", "poetry"),
            ("pdm.lock", "pdm"),
            ("Pipfile.lock", "pipenv"),
            ("hatch.toml", "hatch"),
            ("requirements.txt", "python"),
        ] {
            let direct = TempDir::new().expect("direct tempdir");
            fs::write(direct.path().join(marker), "").expect("manager marker");
            let resolution = adapter
                .resolve_execution(direct.path(), direct.path())
                .expect("direct resolution");
            assert_eq!(resolution.runner, manager);
            assert_eq!(resolution.kind, "direct_root");
            assert_eq!(resolution.install_cwd, direct.path());
            assert_eq!(resolution.exec_cwd, direct.path());
        }

        fs::write(dir.path().join("libs/math/poetry.lock"), "").expect("direct poetry");
        let ambiguous = adapter
            .resolve_execution(dir.path(), &dir.path().join("libs/math"))
            .expect("ambiguous direct resolution");
        assert_eq!(ambiguous.runner, "poetry");
        assert_eq!(ambiguous.kind, "direct_root");
        assert!(ambiguous.ambiguous);
        assert_eq!(ambiguous.install_cwd, dir.path().join("libs/math"));

        let fallback = TempDir::new().expect("fallback tempdir");
        fs::write(fallback.path().join("Pipfile"), "").expect("pipfile");
        let resolution = adapter
            .resolve_execution(fallback.path(), fallback.path())
            .expect("fallback resolution");
        assert_eq!(resolution.runner, "python");
        assert_eq!(resolution.kind, "fallback");
        assert!(!resolution.ambiguous);
    }

    #[test]
    fn declares_honest_selector_support() {
        let adapter = PythonAdapter::new();
        assert_eq!(
            adapter.verification_selector_support(SignalKind::Test),
            VerificationSelectorSupport::new(true, true, true)
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
    fn policy_effectiveness_declares_required_complexity_threshold() {
        let adapter = PythonAdapter::new();
        assert_eq!(
            adapter
                .policy_effectiveness_facts()
                .required_complexity_thresholds,
            vec![ayni_core::ComplexityThresholdKind::FnCognitive]
        );
    }

    #[test]
    fn finding_size_maps_offender_file_consistently_with_capability() {
        let adapter = PythonAdapter::new();
        let offender = SizeOffender {
            file: String::from("src/api.py"),
            value: 800,
            warn: 400,
            fail: 700,
            level: ayni_core::Level::Fail,
        };
        let target = adapter.verification_target(
            SignalKind::Size,
            &Scope::default(),
            OffenderIdentity::Size(&offender),
        );
        adapter
            .verification_selector_support(SignalKind::Size)
            .validate_target(SignalKind::Size, &target)
            .expect("mapped target must match declared support");
        assert_eq!(target.file.as_deref(), Some("src/api.py"));
    }
}
