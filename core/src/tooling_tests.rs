use super::*;
use crate::{AyniPolicy, LanguageAdapter, SignalCollector, SignalRow};

fn request(ownership: SignalToolOwnership) -> ToolingRequest {
    ToolingRequest::new(
        std::env::current_dir().unwrap(),
        TargetIdentity::new(Language::Node, "apps/web").unwrap(),
        [SignalKind::Test, SignalKind::Coverage],
        ownership,
        [SignalKind::Test],
    )
    .unwrap()
}

fn digest() -> String {
    format!("sha256:{}", "a".repeat(64))
}
fn preimage() -> ToolingPreimage {
    ToolingPreimage::Sha256 { digest: digest() }
}

fn plan() -> ToolingPlan {
    let mut plan = ToolingPlan::empty(request(SignalToolOwnership::Ayni).target().clone());
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
    plan.inputs.push(ToolingInput {
        path: "manifest.json".into(),
        digest: digest(),
    });
    plan.edits.push(ToolingFileEdit {
        path: "manifest.json".into(),
        preimage: preimage(),
        content: "new declaration".into(),
    });
    plan.outputs = vec![
        ToolingOutput {
            path: "manifest.json".into(),
            preimage: preimage(),
        },
        ToolingOutput {
            path: "native.lock".into(),
            preimage: ToolingPreimage::Absent,
        },
    ];
    plan.commands.push(ToolingCommand {
        program: "manager".into(),
        args: vec!["lock".into()],
        cwd: ".".into(),
        environment: BTreeMap::new(),
        outputs: vec!["native.lock".into()],
    });
    plan
}

#[test]
fn ownership_defaults_and_explicit_modes_parse_without_changing_other_policy() {
    assert_eq!(
        AyniPolicy::parse("")
            .unwrap()
            .environment
            .signal_tools
            .ownership,
        SignalToolOwnership::Project
    );
    for (value, expected) in [
        ("project", SignalToolOwnership::Project),
        ("ayni", SignalToolOwnership::Ayni),
    ] {
        let policy = AyniPolicy::parse(&format!(
            "[environment.signal_tools]\nownership = '{value}'"
        ))
        .unwrap();
        assert_eq!(policy.environment.signal_tools.ownership, expected);
    }
    for content in [
        "[environment.signal_tools]\nownership = 'mixed'",
        "[environment.signal_tools]\nowner = 'ayni'",
    ] {
        assert!(AyniPolicy::parse(content).is_err());
    }
}

#[test]
fn requests_reject_relative_roots_escaping_targets_and_non_enabled_defaults() {
    let target = TargetIdentity::new(Language::Rust, ".").unwrap();
    assert!(
        ToolingRequest::new(
            "relative".into(),
            target.clone(),
            [],
            SignalToolOwnership::Ayni,
            []
        )
        .is_err()
    );
    assert!(
        ToolingRequest::new(
            std::env::current_dir().unwrap(),
            target,
            [],
            SignalToolOwnership::Ayni,
            [SignalKind::Test]
        )
        .is_err()
    );
    let bad = TargetIdentity {
        language: Language::Rust,
        root: "../other".into(),
    };
    assert!(
        ToolingRequest::new(
            std::env::current_dir().unwrap(),
            bad,
            [],
            SignalToolOwnership::Ayni,
            []
        )
        .is_err()
    );
    let request = request(SignalToolOwnership::Ayni);
    assert!(request.repo_root().is_absolute());
    assert_eq!(request.enabled_signals().len(), 2);
    assert_eq!(request.default_tool_signals().len(), 1);
}

#[test]
fn normalized_plan_is_stable_and_preserves_command_order() {
    let mut plan = plan();
    plan.edits[0].path = "./manifest.json".into();
    plan.commands[0].cwd = "./".into();
    let mut second = plan.commands[0].clone();
    second.args = vec!["verify-integrity".into()];
    plan.commands.push(second);
    let request = request(SignalToolOwnership::Ayni);
    plan.normalize_and_validate(&request).unwrap();
    let first = serde_json::to_string(&plan).unwrap();
    plan.normalize_and_validate(&request).unwrap();
    assert_eq!(first, serde_json::to_string(&plan).unwrap());
    assert_eq!(plan.edits[0].path, "manifest.json");
    assert_eq!(plan.commands[0].args, ["lock"]);
    assert_eq!(plan.commands[1].args, ["verify-integrity"]);
}

#[test]
fn project_owned_and_overridden_signals_cannot_propose_writes() {
    assert!(
        plan()
            .normalize_and_validate(&request(SignalToolOwnership::Project))
            .is_err()
    );
    let mut plan = plan();
    plan.tools[0].signals = BTreeSet::from([SignalKind::Coverage]);
    assert!(
        plan.normalize_and_validate(&request(SignalToolOwnership::Ayni))
            .is_err()
    );
    let request = request(SignalToolOwnership::Project);
    ToolingPlan::empty(request.target().clone())
        .normalize_and_validate(&request)
        .unwrap();
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
        plan.outputs[1].path = path.into();
        plan.commands[0].outputs[0] = path.into();
        assert!(
            plan.normalize_and_validate(&request(SignalToolOwnership::Ayni))
                .is_err(),
            "{path:?}"
        );
    }
    let mut plan = plan();
    plan.owner_root = "unrelated".into();
    assert!(
        plan.normalize_and_validate(&request(SignalToolOwnership::Ayni))
            .is_err()
    );
}

#[test]
fn edits_require_allowlisted_outputs_and_consistent_preimages() {
    let request = request(SignalToolOwnership::Ayni);
    let mut changed = plan();
    changed.edits[0].preimage = ToolingPreimage::Absent;
    assert!(changed.normalize_and_validate(&request).is_err());
    let mut changed = plan();
    changed.inputs[0].digest = format!("sha256:{}", "b".repeat(64));
    assert!(changed.normalize_and_validate(&request).is_err());
    let mut changed = plan();
    changed.outputs[1].preimage = preimage();
    assert!(changed.normalize_and_validate(&request).is_err());
    let mut changed = plan();
    changed.edits[0].path = "undeclared".into();
    assert!(changed.normalize_and_validate(&request).is_err());
    let mut changed = plan();
    changed.outputs[0].preimage = ToolingPreimage::Absent;
    assert!(changed.normalize_and_validate(&request).is_err());
}

#[test]
fn duplicate_and_overlapping_files_are_rejected() {
    let request = request(SignalToolOwnership::Ayni);
    let mut changed = plan();
    changed.edits.push(changed.edits[0].clone());
    assert!(changed.normalize_and_validate(&request).is_err());
    let mut changed = plan();
    changed.inputs.push(changed.inputs[0].clone());
    assert!(changed.normalize_and_validate(&request).is_err());
    let mut changed = plan();
    changed.outputs.push(changed.outputs[0].clone());
    assert!(changed.normalize_and_validate(&request).is_err());
    let mut changed = plan();
    changed.outputs[1].path = "manifest.json/nested".into();
    assert!(changed.normalize_and_validate(&request).is_err());
}

#[test]
fn preimage_digests_are_strict_and_creation_is_explicit() {
    for invalid in [
        "",
        "sha256:a",
        "sha1:abcdef",
        &format!("sha256:{}", "A".repeat(64)),
    ] {
        let mut plan = plan();
        plan.inputs[0].digest = invalid.into();
        assert!(
            plan.normalize_and_validate(&request(SignalToolOwnership::Ayni))
                .is_err()
        );
    }
    let mut plan = plan();
    plan.inputs.clear();
    plan.outputs[0].preimage = ToolingPreimage::Absent;
    plan.edits[0].preimage = ToolingPreimage::Absent;
    plan.normalize_and_validate(&request(SignalToolOwnership::Ayni))
        .unwrap();
}

#[test]
fn command_data_is_validated_without_execution() {
    let request = request(SignalToolOwnership::Ayni);
    for program in [
        "",
        "manager lock",
        "/bin/manager",
        "../manager",
        "manager\0",
        "-manager",
        ".",
        "..",
        "manager;other",
        "a\\b",
    ] {
        let mut plan = plan();
        plan.commands[0].program = program.into();
        assert!(
            plan.normalize_and_validate(&request).is_err(),
            "{program:?}"
        );
    }
    let mut invalid = plan();
    invalid.commands[0].args.push("bad\0arg".into());
    assert!(invalid.normalize_and_validate(&request).is_err());
    let mut invalid = plan();
    invalid.commands[0]
        .environment
        .insert("BAD=KEY".into(), "value".into());
    assert!(invalid.normalize_and_validate(&request).is_err());
    let mut invalid = plan();
    invalid.commands[0].outputs = vec!["undeclared".into()];
    assert!(invalid.normalize_and_validate(&request).is_err());
    let mut invalid = plan();
    invalid.commands[0].cwd = "../outside".into();
    assert!(invalid.normalize_and_validate(&request).is_err());
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
    let request = request(SignalToolOwnership::Ayni);
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
        SignalToolOwnership::Ayni,
        [SignalKind::Test],
    )
    .unwrap();
    assert!(adapter.plan_tooling(&wrong_request).is_err());
}
