use crate::catalog::RUST_CATALOG;
use crate::collectors::RustCollector;
use crate::discovery;
use crate::environment::RustEnvironmentCapability;
use crate::environment_resolution::RustEnvironmentResolutionCapability;
use crate::impact::RustImpactCapability;
use crate::preparation::RustDependencyPreparationCapability;
use ayni_adapters_common::finding::{DependencySource, target_for_finding};
use ayni_core::{
    CatalogEntry, ComplexityThresholdKind, DetectResult, ExecutionResolution, Language,
    LanguageAdapter, LanguageProfile, OffenderIdentity, PolicyEffectivenessFacts, ProjectDiscovery,
    Scope, SignalCollector, SignalKind, VerificationSelectorSupport, VerificationTarget,
};
use std::path::Path;

#[derive(Debug, Default)]
pub struct RustAdapter {
    collector: RustCollector,
}

impl RustAdapter {
    #[must_use]
    pub fn new() -> Self {
        Self {
            collector: RustCollector,
        }
    }
}

impl LanguageAdapter for RustAdapter {
    fn language(&self) -> Language {
        Language::Rust
    }

    fn detect(&self, root: &Path) -> DetectResult {
        let detected = root.join("Cargo.toml").is_file();
        DetectResult {
            detected,
            confidence: if detected { 100 } else { 0 },
            reason: if detected {
                Some(format!("Cargo.toml found at {}", root.display()))
            } else {
                Some(format!("Cargo.toml not found at {}", root.display()))
            },
        }
    }

    fn resolve_execution(&self, repo_root: &Path, root: &Path) -> Option<ExecutionResolution> {
        if !root.join("Cargo.toml").is_file() {
            return None;
        }
        if let Some(workspace) = find_cargo_workspace_ancestor(repo_root, root) {
            return Some(ExecutionResolution {
                runner: String::from("cargo"),
                resolved_from: workspace.clone(),
                kind: String::from("workspace_ancestor"),
                source: String::from("Cargo workspace"),
                confidence: 90,
                ambiguous: false,
                install_cwd: workspace.clone(),
                exec_cwd: workspace,
                environment: std::collections::BTreeMap::new(),
            });
        }
        Some(ExecutionResolution::direct(
            "cargo",
            root.to_path_buf(),
            "Cargo.toml",
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
            language: Language::Rust,
            default_file_globs: vec![String::from("*.rs")],
        }
    }

    fn catalog(&self) -> &'static [CatalogEntry] {
        RUST_CATALOG
    }

    fn impact_capability(&self) -> Option<&dyn ayni_core::ImpactCapability> {
        Some(&RustImpactCapability)
    }

    fn environment_capability(&self) -> Option<&dyn ayni_core::EnvironmentCapability> {
        Some(&RustEnvironmentCapability)
    }

    fn dependency_preparation_capability(
        &self,
    ) -> Option<&dyn ayni_core::DependencyPreparationCapability> {
        Some(&RustDependencyPreparationCapability)
    }

    fn environment_resolution_capability(
        &self,
    ) -> Option<&dyn ayni_core::EnvironmentResolutionCapability> {
        Some(&RustEnvironmentResolutionCapability)
    }

    fn collector(&self) -> &dyn SignalCollector {
        &self.collector
    }

    fn policy_effectiveness_facts(&self) -> PolicyEffectivenessFacts {
        PolicyEffectivenessFacts::new(Language::Rust, vec![ComplexityThresholdKind::FnCyclomatic])
    }

    fn verification_selector_support(&self, kind: SignalKind) -> VerificationSelectorSupport {
        match kind {
            SignalKind::Test => VerificationSelectorSupport::new(false, true, true),
            SignalKind::Complexity | SignalKind::Deps => {
                VerificationSelectorSupport::new(true, true, false)
            }
            SignalKind::Size => VerificationSelectorSupport::new(true, false, false),
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

    /// Cargo serializes builds on the target-directory lock, so running
    /// multiple Rust targets in parallel only causes lock contention.
    fn max_target_concurrency(&self) -> Option<usize> {
        Some(1)
    }
}

fn find_cargo_workspace_ancestor(repo_root: &Path, root: &Path) -> Option<std::path::PathBuf> {
    let mut current = root.parent();
    while let Some(path) = current {
        if !path.starts_with(repo_root) {
            break;
        }
        let manifest = path.join("Cargo.toml");
        if manifest.is_file() && cargo_manifest_has_workspace(&manifest) {
            return Some(path.to_path_buf());
        }
        current = path.parent();
    }
    None
}

fn cargo_manifest_has_workspace(path: &Path) -> bool {
    std::fs::read_to_string(path)
        .ok()
        .is_some_and(|content| content.lines().any(|line| line.trim() == "[workspace]"))
}

#[cfg(test)]
mod tests {
    use super::RustAdapter;
    use ayni_core::{
        DepsOffender, LanguageAdapter, Level, OffenderIdentity, Scope, SignalKind,
        VerificationSelectorSupport,
    };
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn resolves_cargo_workspace_ancestor() {
        let dir = TempDir::new().expect("tempdir");
        fs::write(
            dir.path().join("Cargo.toml"),
            r#"[workspace]
members = ["crates/api"]
"#,
        )
        .expect("workspace manifest");
        fs::create_dir_all(dir.path().join("crates/api")).expect("api dir");
        fs::write(
            dir.path().join("crates/api/Cargo.toml"),
            r#"[package]
name = "api"
version = "0.1.0"
edition = "2021"
"#,
        )
        .expect("api manifest");

        let adapter = RustAdapter::new();
        let resolution = adapter
            .resolve_execution(dir.path(), &dir.path().join("crates/api"))
            .expect("resolution");

        assert_eq!(resolution.runner, "cargo");
        assert_eq!(resolution.kind, "workspace_ancestor");
        assert_eq!(resolution.exec_cwd, dir.path());
    }

    #[test]
    fn declares_honest_selector_support() {
        let adapter = RustAdapter::new();
        assert_eq!(
            adapter.verification_selector_support(SignalKind::Test),
            VerificationSelectorSupport::new(false, true, true)
        );
        assert_eq!(
            adapter.verification_selector_support(SignalKind::Complexity),
            VerificationSelectorSupport::new(true, true, false)
        );
        assert_eq!(
            adapter.verification_selector_support(SignalKind::Deps),
            VerificationSelectorSupport::new(true, true, false)
        );
        assert_eq!(
            adapter.verification_selector_support(SignalKind::Size),
            VerificationSelectorSupport::new(true, false, false)
        );
        for kind in [SignalKind::Coverage, SignalKind::Mutation] {
            assert_eq!(
                adapter.verification_selector_support(kind),
                VerificationSelectorSupport::NONE
            );
        }
    }

    #[test]
    fn policy_effectiveness_declares_required_complexity_threshold() {
        let adapter = RustAdapter::new();
        assert_eq!(
            adapter
                .policy_effectiveness_facts()
                .required_complexity_thresholds,
            vec![ayni_core::ComplexityThresholdKind::FnCyclomatic]
        );
    }

    #[test]
    fn finding_dependency_maps_source_package_consistently_with_capability() {
        let adapter = RustAdapter::new();
        let offender = DepsOffender {
            from: String::from("crates/api"),
            to: String::from("crates/core"),
            rule: String::from("api -> core"),
            level: Level::Fail,
        };
        let target = adapter.verification_target(
            SignalKind::Deps,
            &Scope::default(),
            OffenderIdentity::Deps(&offender),
        );
        adapter
            .verification_selector_support(SignalKind::Deps)
            .validate_target(SignalKind::Deps, &target)
            .expect("mapped target must match declared support");
        assert_eq!(target.package.as_deref(), Some("crates/api"));
    }
}
