use crate::catalog::{NODE_CATALOG, NODE_CATALOG_RUNTIME};
use crate::collectors::NodeCollector;
use crate::discovery;
use crate::environment::NodeEnvironmentCapability;
use crate::environment_resolution::NodeEnvironmentResolutionCapability;
use crate::package_manager;
use crate::preparation::NodeDependencyPreparationCapability;
use ayni_adapters_common::finding::{DependencySource, target_for_finding};
use ayni_core::{
    CatalogEntry, CatalogRuntime, ComplexityThresholdKind, DetectResult, ExecutionResolution,
    Language, LanguageAdapter, LanguageProfile, OffenderIdentity, PolicyEffectivenessFacts,
    ProjectDiscovery, Scope, SignalCollector, SignalKind, VerificationSelectorSupport,
    VerificationTarget,
};
use std::path::Path;

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

        let pm = package_manager::detect(root);
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

    fn catalog_runtime(&self) -> &dyn CatalogRuntime {
        &NODE_CATALOG_RUNTIME
    }

    fn collector(&self) -> &dyn SignalCollector {
        &self.collector
    }

    fn environment_capability(&self) -> Option<&dyn ayni_core::EnvironmentCapability> {
        static CAPABILITY: NodeEnvironmentCapability = NodeEnvironmentCapability;
        Some(&CAPABILITY)
    }

    fn dependency_preparation_capability(
        &self,
    ) -> Option<&dyn ayni_core::DependencyPreparationCapability> {
        static CAPABILITY: NodeDependencyPreparationCapability =
            NodeDependencyPreparationCapability;
        Some(&CAPABILITY)
    }

    fn environment_resolution_capability(
        &self,
    ) -> Option<&dyn ayni_core::EnvironmentResolutionCapability> {
        static CAPABILITY: NodeEnvironmentResolutionCapability =
            NodeEnvironmentResolutionCapability;
        Some(&CAPABILITY)
    }

    fn policy_effectiveness_facts(&self) -> PolicyEffectivenessFacts {
        PolicyEffectivenessFacts::new(Language::Node, vec![ComplexityThresholdKind::FnCyclomatic])
    }

    fn verification_selector_support(&self, kind: SignalKind) -> VerificationSelectorSupport {
        match kind {
            SignalKind::Test => VerificationSelectorSupport::new(true, true, true),
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

#[cfg(test)]
mod tests {
    use super::NodeAdapter;
    use ayni_core::{
        LanguageAdapter, OffenderIdentity, Scope, SignalKind, TestFailure,
        VerificationSelectorSupport,
    };
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn package_manager_resolution_matrix() {
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

        fs::write(
            dir.path().join("apps/api/package.json"),
            r#"{"packageManager":"yarn@4"}"#,
        )
        .expect("direct manager");
        let direct = adapter
            .resolve_execution(dir.path(), &dir.path().join("apps/api"))
            .expect("direct resolution");
        assert_eq!(direct.runner, "yarn");
        assert_eq!(direct.kind, "direct_root");
        assert!(direct.ambiguous);
        assert_eq!(direct.install_cwd, dir.path().join("apps/api"));
        assert_eq!(direct.exec_cwd, dir.path().join("apps/api"));

        let fallback_dir = TempDir::new().expect("fallback tempdir");
        let standalone = fallback_dir.path().join("standalone");
        fs::create_dir_all(&standalone).expect("standalone dir");
        fs::write(standalone.join("package.json"), "{}").expect("standalone manifest");
        let fallback = adapter
            .resolve_execution(fallback_dir.path(), &standalone)
            .expect("fallback resolution");
        assert_eq!(fallback.runner, "npm");
        assert_eq!(fallback.kind, "fallback");
        assert!(!fallback.ambiguous);
        assert_eq!(fallback.install_cwd, standalone);
        assert_eq!(fallback.exec_cwd, fallback.install_cwd);

        for manager in ["npm", "pnpm", "yarn", "bun"] {
            let direct_dir = TempDir::new().expect("direct tempdir");
            fs::write(
                direct_dir.path().join("package.json"),
                format!(r#"{{"packageManager":"{manager}@1"}}"#),
            )
            .expect("direct manifest");
            let resolution = adapter
                .resolve_execution(direct_dir.path(), direct_dir.path())
                .expect("direct resolution");
            assert_eq!(resolution.runner, manager);
            assert_eq!(resolution.kind, "direct_root");
            assert_eq!(resolution.install_cwd, direct_dir.path());
            assert_eq!(resolution.exec_cwd, direct_dir.path());
        }

        let npm_workspace = TempDir::new().expect("npm workspace tempdir");
        fs::write(
            npm_workspace.path().join("package.json"),
            r#"{"workspaces":["packages/*"]}"#,
        )
        .expect("workspace manifest");
        let member = npm_workspace.path().join("packages/member");
        fs::create_dir_all(&member).expect("member directory");
        fs::write(member.join("package.json"), "{}").expect("member manifest");
        let workspace_fallback = adapter
            .resolve_execution(npm_workspace.path(), &member)
            .expect("workspace npm resolution");
        assert_eq!(workspace_fallback.runner, "npm");
        assert_eq!(workspace_fallback.kind, "workspace_ancestor");
        assert_eq!(workspace_fallback.install_cwd, npm_workspace.path());
        assert_eq!(workspace_fallback.exec_cwd, member);
    }

    #[test]
    fn declares_honest_selector_support() {
        let adapter = NodeAdapter::new();
        assert_eq!(
            adapter.verification_selector_support(SignalKind::Test),
            VerificationSelectorSupport::new(true, true, true)
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
    fn finding_test_maps_file_and_name_when_both_are_supported() {
        let adapter = NodeAdapter::new();
        let offender = TestFailure {
            file: Some(String::from("tests/api.test.ts")),
            line: Some(10),
            message: String::from("failed"),
            test_name: Some(String::from("creates user")),
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
        assert_eq!(target.file, offender.file);
        assert_eq!(target.name, offender.test_name);
    }
}
