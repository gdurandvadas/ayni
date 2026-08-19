use super::*;
use crate::language::Language;
use crate::runtime::Scope;

#[test]
fn run_artifact_json_roundtrip_preserves_rows() {
    let mut artifact = RunArtifact::new(
        RunArtifactMetadata {
            generated_at: String::from("2026-07-12T00:00:00Z"),
            ayni_version: String::from("0.4.2"),
            invocation: InvocationContext {
                command: String::from("analyze"),
                languages: vec![Language::Rust],
                scope: None,
            },
            output: OutputContext {
                format: String::from("json"),
                destination: String::from("stdout"),
            },
            config_path: String::from(".ayni.toml"),
            repository_root: String::from("."),
            ..RunArtifactMetadata::default()
        },
        RunCompletion::complete(CompletionScope::Repository, 1),
        vec![SignalRow {
            kind: SignalKind::Test,
            language: Language::Rust,
            scope: Scope {
                workspace_root: String::from("."),
                path: Some(String::from("crates/api")),
                package: None,
                file: None,
            },
            pass: false,
            result: SignalResult::Test(TestResult {
                total_tests: 10,
                passed: 9,
                failed: 1,
                duration_ms: Some(1234),
                runner: String::from("cargo test"),
                failure: Some(CommandFailure {
                    category: String::from("repo_code_issue"),
                    classification: String::from("command_error"),
                    command: String::from("cargo test"),
                    cwd: String::from("."),
                    exit_code: Some(101),
                    message: String::from("1 test failed"),
                }),
            }),
            budget: Budget::Test(TestBudget::default()),
            offenders: Offenders::Test(vec![TestFailure {
                file: Some(String::from("src/lib.rs")),
                line: Some(42),
                message: String::from("assertion failed"),
                test_name: Some(String::from("does_thing")),
            }]),
        }],
    )
    .expect("valid staged artifact");
    materialize_findings(&mut artifact);

    let serialized = serde_json::to_string_pretty(&artifact).expect("serialize");
    let deserialized = serde_json::from_str::<RunArtifact>(&serialized).expect("deserialize");
    assert_eq!(deserialized, artifact);

    let value: serde_json::Value = serde_json::from_str(&serialized).expect("json value");
    assert_eq!(value["schema_version"], AYNI_SIGNAL_SCHEMA_VERSION);
    assert_eq!(value["generated_at"], "2026-07-12T00:00:00Z");
    assert_eq!(value["aggregate"]["status"], "fail");
    assert_eq!(value["aggregate"]["total_rows"], 1);
    assert_eq!(value["aggregate"]["failing_offenders"], 1);
    assert_eq!(value["applied_thresholds"][0]["kind"], "test");
    assert_eq!(value["offender_summaries"][0]["failing_count"], 1);
    assert_eq!(
        value["failure_summaries"][0]["classification"],
        "command_error"
    );
    assert_eq!(value["failure_summaries"][0]["exit_code"], 101);
    assert_eq!(value["rows"][0]["kind"], "test");
    assert_eq!(value["rows"][0]["offenders"]["kind"], "test");
}

#[test]
fn derived_summaries_are_deterministic_and_empty_failures_are_omitted() {
    let row = SignalRow {
        kind: SignalKind::Size,
        language: Language::Rust,
        scope: Scope::default(),
        pass: true,
        result: SignalResult::Size(SizeResult {
            max_lines: 20,
            total_files: 1,
            warn_count: 1,
            fail_count: 0,
            failure: None,
        }),
        budget: Budget::Size(SizeBudget {
            warn: Some(10),
            fail: Some(30),
            ..SizeBudget::default()
        }),
        offenders: Offenders::Size(vec![SizeOffender {
            file: String::from("src/lib.rs"),
            value: 20,
            warn: 10,
            fail: 30,
            level: Level::Warn,
        }]),
    };
    let mut artifact = RunArtifact::new(
        RunArtifactMetadata::default(),
        RunCompletion::complete(CompletionScope::Repository, 1),
        vec![row],
    )
    .expect("valid staged artifact");
    materialize_findings(&mut artifact);

    assert_eq!(artifact.aggregate().status, AggregateStatus::Pass);
    assert_eq!(artifact.aggregate().warning_offenders, 1);
    assert_eq!(
        artifact.applied_thresholds()[0].budget,
        Budget::Size(SizeBudget {
            warn: Some(10),
            fail: Some(30),
            ..SizeBudget::default()
        })
    );
    assert_eq!(artifact.offender_summaries()[0].warning_count, 1);
    assert_eq!(artifact.failure_summaries(), None);

    let value = serde_json::to_value(&artifact).expect("serialize");
    assert!(value.get("failure_summaries").is_none());
    assert!(serde_json::from_value::<RunArtifact>(value).is_ok());
}

#[test]
fn size_and_deps_failures_roundtrip_to_complete_failure_summaries() {
    let failure = |kind: &str, exit_code| CommandFailure {
        category: format!("{kind}_category"),
        classification: format!("{kind}_classification"),
        command: format!("{kind} command"),
        cwd: format!("/{kind}"),
        exit_code,
        message: format!("{kind} message"),
    };
    let artifact = RunArtifact::new(
        RunArtifactMetadata::default(),
        RunCompletion::complete(CompletionScope::Repository, 1),
        vec![
            SignalRow {
                kind: SignalKind::Size,
                language: Language::Rust,
                scope: Scope::default(),
                pass: false,
                result: SignalResult::Size(SizeResult {
                    max_lines: 0,
                    total_files: 0,
                    warn_count: 0,
                    fail_count: 1,
                    failure: Some(failure("size", Some(17))),
                }),
                budget: Budget::Size(SizeBudget::default()),
                offenders: Offenders::Size(Vec::new()),
            },
            SignalRow {
                kind: SignalKind::Deps,
                language: Language::Rust,
                scope: Scope::default(),
                pass: false,
                result: SignalResult::Deps(DepsResult {
                    crate_count: 0,
                    edge_count: 0,
                    violation_count: 1,
                    failure: Some(failure("deps", None)),
                }),
                budget: Budget::Deps(DepsBudget::default()),
                offenders: Offenders::Deps(Vec::new()),
            },
        ],
    )
    .expect("valid artifact");

    assert_eq!(
        artifact.rows[0].result.command_failure(),
        Some(&failure("size", Some(17)))
    );
    assert_eq!(
        artifact.rows[1].result.command_failure(),
        Some(&failure("deps", None))
    );
    let summaries = artifact.failure_summaries().expect("failure summaries");
    for (summary, kind, exit_code) in [
        (&summaries[0], "size", Some(17)),
        (&summaries[1], "deps", None),
    ] {
        assert_eq!(summary.category, format!("{kind}_category"));
        assert_eq!(summary.classification, format!("{kind}_classification"));
        assert_eq!(summary.command, format!("{kind} command"));
        assert_eq!(summary.cwd, format!("/{kind}"));
        assert_eq!(summary.exit_code, exit_code);
        assert_eq!(summary.message, format!("{kind} message"));
    }

    let serialized = serde_json::to_string(&artifact).expect("serialize");
    assert_eq!(
        serde_json::from_str::<RunArtifact>(&serialized).expect("roundtrip"),
        artifact
    );
}

#[test]
fn serialization_rejects_mismatched_row_payload_variants() {
    let mut row = structural_test_row(".", SignalKind::Size);
    row.kind = SignalKind::Test;
    let error = RunArtifact::new(
        RunArtifactMetadata::default(),
        RunCompletion::complete(CompletionScope::Repository, 1),
        vec![row],
    )
    .expect_err("mismatched payloads");
    assert!(error.to_string().contains("typed payloads"), "{error}");
}

#[test]
fn serialization_rejects_unknown_budget_fields() {
    let artifact = RunArtifact::new(
        RunArtifactMetadata::default(),
        RunCompletion::complete(CompletionScope::Repository, 1),
        vec![structural_test_row(".", SignalKind::Size)],
    )
    .expect("valid artifact");
    let mut value = serde_json::to_value(artifact).expect("artifact value");
    value["rows"][0]["budget"]["invented"] = serde_json::json!(1);
    value["applied_thresholds"][0]["budget"]["invented"] = serde_json::json!(1);
    let error = serde_json::from_value::<RunArtifact>(value).expect_err("unknown budget field");
    assert!(
        error.to_string().contains("unknown field `invented`"),
        "{error}"
    );
}

#[test]
fn serialization_requires_findings_for_every_offender_row() {
    let artifact = RunArtifact::new(
        RunArtifactMetadata::default(),
        RunCompletion::complete(CompletionScope::Repository, 1),
        vec![failing_test_row()],
    )
    .expect("valid staged artifact");

    let error = serde_json::to_value(artifact).expect_err("missing findings");
    assert!(
        error.to_string().contains("require one finding collection"),
        "{error}"
    );
}

#[test]
fn serialization_rejects_short_or_mismatched_finding_collections() {
    let mut artifact = RunArtifact::new(
        RunArtifactMetadata::default(),
        RunCompletion::complete(CompletionScope::Repository, 1),
        vec![
            failing_test_row(),
            structural_test_row(".", SignalKind::Size),
        ],
    )
    .expect("valid staged artifact");
    artifact.findings = vec![wire_findings(&artifact.rows[0])];
    let error = serde_json::to_value(&artifact).expect_err("short findings");
    assert!(error.to_string().contains("one-for-one"), "{error}");

    artifact.findings = vec![Findings::Size(Vec::new()), Findings::Size(Vec::new())];
    let error = serde_json::to_value(&artifact).expect_err("mismatched kind");
    assert!(error.to_string().contains("does not match"), "{error}");

    artifact.findings = vec![
        wire_findings(&artifact.rows[0]),
        wire_findings(&artifact.rows[1]),
    ];
    let Findings::Test(items) = &mut artifact.findings[0] else {
        panic!("test findings");
    };
    items[0].offender.message = String::from("different offender");
    let error = serde_json::to_value(&artifact).expect_err("mismatched offender");
    assert!(error.to_string().contains("does not match"), "{error}");
}

#[test]
fn schema_v4_rejects_legacy_offenders_and_staged_targets() {
    let mut artifact = RunArtifact::new(
        RunArtifactMetadata::default(),
        RunCompletion::complete(CompletionScope::Repository, 1),
        vec![failing_test_row()],
    )
    .expect("valid staged artifact");

    artifact.findings = artifact
        .rows
        .iter()
        .map(|row| {
            row.offenders
                .clone()
                .into_findings(row.language, &row.scope.workspace_root, |_| {
                    VerificationTarget::default()
                })
                .expect("finding identities")
        })
        .collect();
    let error = serde_json::to_value(&artifact).expect_err("staged target");
    assert!(
        error.to_string().contains("rendered verification"),
        "{error}"
    );

    materialize_findings(&mut artifact);
    let mut value = serde_json::to_value(&artifact).expect("schema-v4 artifact");
    value["rows"][0]["offenders"] =
        serde_json::to_value(&artifact.rows[0].offenders).expect("legacy offenders");
    let error = serde_json::from_value::<RunArtifact>(value).expect_err("legacy offenders");
    assert!(error.to_string().contains("schema-v4 findings"), "{error}");
}

#[test]
fn managed_artifact_requires_lock_provenance() {
    let error = RunArtifact::new(
        RunArtifactMetadata {
            execution_mode: ExecutionMode::Managed,
            environment_lock_fingerprint: None,
            ..RunArtifactMetadata::default()
        },
        RunCompletion::complete(CompletionScope::Repository, 1),
        vec![structural_test_row(".", SignalKind::Size)],
    )
    .expect_err("missing managed lock");
    assert!(
        error.to_string().contains("environment_lock_fingerprint"),
        "{error}"
    );
}

#[test]
fn incomplete_artifact_fails_aggregate_even_with_no_rows() {
    let artifact = RunArtifact::new(
        RunArtifactMetadata::default(),
        RunCompletion {
            scope: CompletionScope::Requested,
            state: CompletionState::Incomplete,
            expected_targets: 1,
            detected_targets: 0,
            completed_targets: 0,
            skipped_targets: 1,
            issues: vec![CompletionIssue {
                language: Language::Rust,
                configured_root: String::from("."),
                stage: CompletionStage::Detection,
                message: String::from("configured target was not detected"),
            }],
        },
        Vec::new(),
    )
    .expect("valid artifact");

    assert_eq!(artifact.aggregate().status, AggregateStatus::Fail);
    let value = serde_json::to_value(&artifact).expect("serialize");
    assert_eq!(value["completion"]["scope"], "requested");
    assert_eq!(value["completion"]["state"], "incomplete");
    assert_eq!(value["aggregate"]["status"], "fail");
    assert_eq!(
        serde_json::from_value::<RunArtifact>(value).expect("roundtrip"),
        artifact
    );
}

#[test]
fn deserialization_rejects_unreconciled_completion() {
    let artifact = RunArtifact::new(
        RunArtifactMetadata::default(),
        RunCompletion::complete(CompletionScope::Repository, 0),
        Vec::new(),
    )
    .expect("valid artifact");
    let mut value = serde_json::to_value(artifact).expect("serialize");
    value["completion"]["state"] = serde_json::json!("incomplete");

    let error = serde_json::from_value::<RunArtifact>(value).expect_err("invalid completion");
    assert!(error.to_string().contains("incomplete artifact"));
}

#[test]
fn complete_artifact_requires_rows() {
    let artifact = RunArtifact::new(
        RunArtifactMetadata::default(),
        RunCompletion::complete(CompletionScope::Repository, 1),
        vec![structural_test_row(".", SignalKind::Size)],
    )
    .expect("valid artifact");
    let mut value = serde_json::to_value(artifact).expect("valid artifact");
    value["rows"] = serde_json::json!([]);
    value["applied_thresholds"] = serde_json::json!([]);
    value["offender_summaries"] = serde_json::json!([]);
    value["aggregate"] = serde_json::json!({
        "status": "fail",
        "total_rows": 0,
        "passing_rows": 0,
        "failing_rows": 0,
        "warning_offenders": 0,
        "failing_offenders": 0
    });

    let error = serde_json::from_value::<RunArtifact>(value).expect_err("missing rows");
    assert!(error.to_string().contains("must contain rows"), "{error}");
}

#[test]
fn complete_artifact_rejects_inconsistent_row_sets() {
    let artifact = RunArtifact::new(
        RunArtifactMetadata::default(),
        RunCompletion::complete(CompletionScope::Repository, 2),
        vec![
            structural_test_row(".", SignalKind::Size),
            structural_test_row("crate", SignalKind::Size),
        ],
    )
    .expect("valid artifact");
    let value = serde_json::to_value(artifact).expect("valid artifact");

    let mut duplicate = value.clone();
    let row = duplicate["rows"][0].clone();
    duplicate["rows"].as_array_mut().expect("rows").push(row);
    let error =
        serde_json::from_value::<RunArtifact>(duplicate).expect_err("duplicate completion key");
    assert!(error.to_string().contains("must be unique"), "{error}");

    let mut missing_target = value.clone();
    missing_target["rows"].as_array_mut().expect("rows").pop();
    let error = serde_json::from_value::<RunArtifact>(missing_target)
        .expect_err("represented target mismatch");
    assert!(
        error.to_string().contains("fewer targets")
            || error.to_string().contains("do not reconcile"),
        "{error}"
    );

    let mut inconsistent_kinds = value;
    inconsistent_kinds["rows"][1]["kind"] = serde_json::json!("test");
    inconsistent_kinds["rows"][1]["result"] = serde_json::json!({
        "kind": "test",
        "total_tests": 1,
        "passed": 1,
        "failed": 0,
        "runner": "test"
    });
    inconsistent_kinds["rows"][1]["budget"] = serde_json::json!({"kind": "test"});
    inconsistent_kinds["rows"][1]["offenders"] = serde_json::json!({"kind": "test", "items": []});
    let error = serde_json::from_value::<RunArtifact>(inconsistent_kinds)
        .expect_err("inconsistent signal kinds");
    assert!(
        error
            .to_string()
            .contains("consistent non-empty signal-kind set"),
        "{error}"
    );
}

fn structural_test_row(root: &str, kind: SignalKind) -> SignalRow {
    assert_eq!(kind, SignalKind::Size);
    SignalRow {
        kind,
        language: Language::Rust,
        scope: Scope {
            workspace_root: String::from("."),
            path: (root != ".").then(|| root.to_string()),
            package: None,
            file: None,
        },
        pass: true,
        result: SignalResult::Size(SizeResult {
            max_lines: 0,
            total_files: 0,
            warn_count: 0,
            fail_count: 0,
            failure: None,
        }),
        budget: Budget::Size(SizeBudget::default()),
        offenders: Offenders::Size(Vec::new()),
    }
}

fn failing_test_row() -> SignalRow {
    SignalRow {
        kind: SignalKind::Test,
        language: Language::Rust,
        scope: Scope::default(),
        pass: false,
        result: SignalResult::Test(TestResult {
            total_tests: 1,
            passed: 0,
            failed: 1,
            duration_ms: None,
            runner: String::from("cargo test"),
            failure: None,
        }),
        budget: Budget::Test(TestBudget::default()),
        offenders: Offenders::Test(vec![TestFailure {
            file: Some(String::from("tests/example.rs")),
            line: Some(7),
            message: String::from("failed"),
            test_name: Some(String::from("example")),
        }]),
    }
}

fn wire_findings(row: &SignalRow) -> Findings {
    let mut findings = row
        .offenders
        .clone()
        .into_findings(row.language, &row.scope.workspace_root, |_| {
            VerificationTarget::default()
        })
        .expect("finding identities");
    findings
        .render_commands(|_| Ok(String::from("ayni verify test")))
        .expect("render finding command");
    findings
}

fn materialize_findings(artifact: &mut RunArtifact) {
    artifact.findings = artifact.rows.iter().map(wire_findings).collect();
}

#[test]
fn deserialization_rejects_historical_schema() {
    let artifact = RunArtifact::new(
        RunArtifactMetadata::default(),
        RunCompletion::complete(CompletionScope::Repository, 0),
        Vec::new(),
    )
    .expect("valid artifact");
    let mut value = serde_json::to_value(artifact).expect("serialize");
    value["schema_version"] = serde_json::json!("0.2.0");

    let error = serde_json::from_value::<RunArtifact>(value).expect_err("historical schema");
    assert!(
        error
            .to_string()
            .contains("unsupported artifact schema_version")
    );
}
