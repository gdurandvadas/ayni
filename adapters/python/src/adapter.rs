use crate::catalog::PYTHON_CATALOG;
use crate::collectors::PythonCollector;
use crate::discovery;
use ayni_adapters_common::finding::{DependencySource, target_for_finding};
use ayni_core::{
    CatalogEntry, ComplexityThresholdKind, DetectResult, ExecutionResolution, Language,
    LanguageAdapter, LanguageProfile, OffenderIdentity, PolicyEffectivenessFacts, ProjectDiscovery,
    Scope, SignalCollector, SignalKind, VerificationSelectorSupport, VerificationTarget,
    detect_python_package_manager, resolve_python_package_manager,
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
