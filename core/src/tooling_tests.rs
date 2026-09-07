use super::*;
use crate::{LanguageAdapter, SignalCollector, SignalRow};

fn request() -> ToolingRequest {
    ToolingRequest::new(
        std::env::current_dir().unwrap(),
        TargetIdentity::new(Language::Node, "apps/web").unwrap(),
        [SignalKind::Test, SignalKind::Coverage],
        [SignalKind::Test],
    )
    .unwrap()
}

fn plan() -> ToolingPlan {
    let mut plan = ToolingPlan::empty(request().target().clone());
    plan.owner_root = ".".into();
    plan.tools.push(ToolingRequirement {
        tool: "runner".into(),
        baseline: VersionRequirement::exact("1.2.3").unwrap(),
        authority: ToolVersionAuthority::AdapterPinned,
        scope: ToolInstallationScope::Project,
        signals: BTreeSet::from([SignalKind::Test]),
        declaration_path: Some("manifest.json".into()),
        current_declaration: None,
        current_resolution: None,
    });
    plan
}
#[test]
fn requests_reject_relative_roots_escaping_targets_and_non_enabled_defaults() {
    let target = TargetIdentity::new(Language::Rust, ".").unwrap();
    assert!(ToolingRequest::new("relative".into(), target.clone(), [], []).is_err());
    assert!(
        ToolingRequest::new(
            std::env::current_dir().unwrap(),
            target,
            [],
            [SignalKind::Test]
        )
        .is_err()
    );
    let bad = TargetIdentity {
        language: Language::Rust,
        root: "../other".into(),
    };
    assert!(ToolingRequest::new(std::env::current_dir().unwrap(), bad, [], []).is_err());
    let request = request();
    assert!(request.repo_root().is_absolute());
    assert_eq!(request.enabled_signals().len(), 2);
    assert_eq!(request.default_tool_signals().len(), 1);
}

#[test]
fn paths_cannot_escape_or_target_repository_control_state() {
    for path in [
        "/tmp/output",
        "../output",
        "nested/../output",
        "C:/output",
        "a\\b",
        "a\0b",
        ".git/config",
        ".GIT/config",
        "line\nbreak",
        ".ayni/state",
        ".ayni.lock",
        ".ayni.toml",
        ".",
        "directory/",
        "directory/.",
    ] {
        let mut plan = plan();
        plan.tools[0].declaration_path = Some(path.into());
        assert!(plan.normalize_and_validate(&request()).is_err(), "{path:?}");
    }
    let mut plan = plan();
    plan.owner_root = "unrelated".into();
    assert!(plan.normalize_and_validate(&request()).is_err());
}

struct TestAdapter {
    capability_language: Language,
    forged: bool,
}
impl SignalCollector for TestAdapter {
    fn collect(&self, _: SignalKind, _: &crate::RunContext) -> Result<SignalRow, AdapterError> {
        unreachable!("planning must not collect")
    }
}
impl ToolingReconciliationCapability for TestAdapter {
    fn language(&self) -> Language {
        self.capability_language
    }
    fn plan(&self, _: &ToolingRequest) -> Result<ToolingPlan, AdapterError> {
        let mut plan = plan();
        if self.forged {
            plan.target.root = "other".into();
        }
        Ok(plan)
    }
}
impl LanguageAdapter for TestAdapter {
    fn language(&self) -> Language {
        Language::Node
    }
    fn detect(&self, _: &Path) -> crate::DetectResult {
        unreachable!("planning must not detect")
    }
    fn discover_roots(&self, _: &Path) -> Vec<String> {
        Vec::new()
    }
    fn profile(&self) -> crate::LanguageProfile {
        crate::LanguageProfile {
            language: Language::Node,
            default_file_globs: Vec::new(),
        }
    }
    fn catalog(&self) -> &'static [crate::CatalogEntry] {
        &[]
    }
    fn collector(&self) -> &dyn SignalCollector {
        self
    }
    fn tooling_reconciliation_capability(&self) -> Option<&dyn ToolingReconciliationCapability> {
        Some(self)
    }
}

#[test]
fn wrapper_validates_capability_identity_and_returned_plan() {
    let request = request();
    let adapter = TestAdapter {
        capability_language: Language::Node,
        forged: false,
    };
    assert!(adapter.plan_tooling(&request).is_ok());
    let wrong_language = TestAdapter {
        capability_language: Language::Go,
        forged: false,
    };
    assert!(wrong_language.plan_tooling(&request).is_err());
    let forged = TestAdapter {
        capability_language: Language::Node,
        forged: true,
    };
    assert!(forged.plan_tooling(&request).is_err());
    let wrong_request = ToolingRequest::new(
        std::env::current_dir().unwrap(),
        TargetIdentity::new(Language::Go, ".").unwrap(),
        [SignalKind::Test],
        [SignalKind::Test],
    )
    .unwrap();
    assert!(adapter.plan_tooling(&wrong_request).is_err());
}

#[test]
fn inspection_rejects_overridden_signals_and_duplicate_tools() {
    let request = request();
    let mut overridden = plan();
    overridden.tools[0].signals = BTreeSet::from([SignalKind::Coverage]);
    assert!(overridden.normalize_and_validate(&request).is_err());
    let mut duplicate = plan();
    duplicate.tools.push(duplicate.tools[0].clone());
    assert!(duplicate.normalize_and_validate(&request).is_err());
}

#[test]
fn inspection_normalizes_paths_and_diagnostics_deterministically() {
    let mut plan = plan();
    plan.tools[0].declaration_path = Some("./manifest.json".into());
    let finding = ToolingDiagnostic {
        code: "tooling.missing".into(),
        message: "Declare runner".into(),
        path: Some("./manifest.json".into()),
    };
    plan.conflicts = vec![finding.clone(), finding];
    plan.normalize_and_validate(&request()).unwrap();
    assert_eq!(plan.conflicts.len(), 1);
    assert_eq!(
        plan.tools[0].declaration_path.as_deref(),
        Some("manifest.json")
    );
    let first = serde_json::to_string(&plan).unwrap();
    plan.normalize_and_validate(&request()).unwrap();
    assert_eq!(first, serde_json::to_string(&plan).unwrap());
}
