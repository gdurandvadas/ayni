use ayni_core::{ArtifactComparison, RunArtifact, compare_artifacts};
use std::fs;
use std::path::Path;
use std::process::ExitCode;

/// Reads only the two paths supplied by the caller and emits either one JSON
/// document or a deterministic human projection of the same comparison model.
pub(crate) fn run(baseline: &Path, candidate: &Path, json: bool) -> ExitCode {
    let baseline = match load_artifact(baseline, "baseline") {
        Ok(artifact) => artifact,
        Err(error) => return fail(error),
    };
    let candidate = match load_artifact(candidate, "candidate") {
        Ok(artifact) => artifact,
        Err(error) => return fail(error),
    };
    let comparison = match compare_artifacts(&baseline, &candidate) {
        Ok(comparison) => comparison,
        Err(error) => return fail(format!("artifact comparison failed: {error}")),
    };

    if json {
        match serde_json::to_string_pretty(&comparison) {
            Ok(document) => println!("{document}"),
            Err(error) => return fail(format!("could not serialize comparison: {error}")),
        }
    } else {
        print!("{}", human_report(&comparison));
    }
    ExitCode::SUCCESS
}

fn load_artifact(path: &Path, side: &str) -> Result<RunArtifact, String> {
    let content = fs::read_to_string(path)
        .map_err(|error| format!("could not read {side} artifact {}: {error}", path.display()))?;
    serde_json::from_str(&content).map_err(|error| {
        format!(
            "could not parse {side} artifact {}: {error}",
            path.display()
        )
    })
}

fn fail(error: String) -> ExitCode {
    eprintln!("{error}");
    ExitCode::FAILURE
}

fn human_report(comparison: &ArtifactComparison) -> String {
    let mut output = format!(
        "# ayni artifact compare\n\ncomparison schema `{}` · artifact schema `{}`\n\nmatched: {} · changed: {} · added: {} · removed: {}\n",
        comparison.comparison_schema_version,
        comparison.artifact_schema_version,
        comparison.matched.len(),
        comparison.changed.len(),
        comparison.added.len(),
        comparison.removed.len(),
    );
    for matched in &comparison.matched {
        let state = if matched.changed {
            "changed"
        } else {
            "unchanged"
        };
        output.push_str(&format!("- {state}: {}\n", row_key(&matched.key)));
        if let Some(pass) = &matched.changes.pass {
            output.push_str(&format!("  pass: {} -> {}\n", pass.before, pass.after));
        }
        for metric in &matched.changes.metrics {
            output.push_str(&format!(
                "  metric {}: {} -> {}\n",
                metric.metric,
                metric.before.as_ref().map_or("absent".into(), metric_value),
                metric.after.as_ref().map_or("absent".into(), metric_value),
            ));
        }
        if let Some(ids) = &matched.changes.finding_ids {
            output.push_str(&format!("  finding IDs added: {}\n", ids.added.join(", ")));
            output.push_str(&format!(
                "  finding IDs removed: {}\n",
                ids.removed.join(", ")
            ));
        }
    }
    for key in &comparison.added {
        output.push_str(&format!("- added: {}\n", row_key(key)));
    }
    for key in &comparison.removed {
        output.push_str(&format!("- removed: {}\n", row_key(key)));
    }
    output
}

fn row_key(key: &ayni_core::SignalRowKey) -> String {
    [
        signal_kind_name(key.kind).to_string(),
        key.language.as_str().to_string(),
        key.path.clone().unwrap_or_else(|| "-".into()),
        key.package.clone().unwrap_or_else(|| "-".into()),
        key.file.clone().unwrap_or_else(|| "-".into()),
    ]
    .join(" / ")
}

fn signal_kind_name(kind: ayni_core::SignalKind) -> &'static str {
    match kind {
        ayni_core::SignalKind::Test => "test",
        ayni_core::SignalKind::Coverage => "coverage",
        ayni_core::SignalKind::Size => "size",
        ayni_core::SignalKind::Complexity => "complexity",
        ayni_core::SignalKind::Deps => "deps",
        ayni_core::SignalKind::Mutation => "mutation",
    }
}

fn metric_value(value: &ayni_core::MetricValue) -> String {
    match value {
        ayni_core::MetricValue::Integer(value) => value.to_string(),
        ayni_core::MetricValue::Float(value) => value.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use ayni_core::{
        ARTIFACT_COMPARISON_SCHEMA_VERSION, ArtifactComparisonError, Budget, CompletionIssue,
        CompletionScope, CompletionStage, CompletionState, Finding, FindingIdChanges,
        FindingMetadata, Findings, InvocationContext, Language, MetricValue, Offenders,
        OutputContext, RunArtifact, RunArtifactMetadata, RunCompletion, Scope, SignalKind,
        SignalResult, SignalRow, TestFailure, TestResult, ValueChange, VerificationMetadata,
        compare_artifacts,
    };

    #[test]
    fn artifact_compare_matches_relative_scope_and_reports_typed_changes() {
        let before = artifact(
            "/checkout/one",
            row(false, 8, 2, "/checkout/one"),
            finding('a'),
        );
        let after = artifact(
            "/different/checkout",
            row(true, 10, 0, "/different/checkout"),
            finding('b'),
        );

        let comparison = compare_artifacts(&before, &after).expect("comparison");

        assert_eq!(
            comparison.comparison_schema_version,
            ARTIFACT_COMPARISON_SCHEMA_VERSION
        );
        assert_eq!(comparison.matched.len(), 1);
        assert_eq!(comparison.changed.len(), 1);
        assert!(comparison.added.is_empty());
        assert!(comparison.removed.is_empty());
        let matched = &comparison.matched[0];
        assert!(matched.changed);
        assert_eq!(
            matched.changes.pass,
            Some(ValueChange {
                before: false,
                after: true
            })
        );
        assert_eq!(matched.changes.metrics[0].metric, "failed");
        assert_eq!(
            matched.changes.metrics[0].before,
            Some(MetricValue::Integer(2))
        );
        assert_eq!(
            matched.changes.finding_ids,
            Some(FindingIdChanges {
                added: vec![id('b')],
                removed: vec![id('a')],
            })
        );
    }

    #[test]
    fn artifact_compare_is_deterministic_and_preserves_ids_after_loading() {
        let before = artifact("/a", row(true, 10, 0, "/a"), finding('a'));
        let after = artifact("/b", row(true, 10, 0, "/b"), finding('a'));
        let before_json = serde_json::to_string(&before).expect("before JSON");
        let after_json = serde_json::to_string(&after).expect("after JSON");
        let loaded_before: RunArtifact = serde_json::from_str(&before_json).expect("before load");
        let loaded_after: RunArtifact = serde_json::from_str(&after_json).expect("after load");

        let direct = compare_artifacts(&before, &after).expect("direct");
        let loaded = compare_artifacts(&loaded_before, &loaded_after).expect("loaded");

        assert_eq!(loaded, direct);
        assert!(!loaded.matched[0].changed);
    }

    #[test]
    fn artifact_compare_classifies_added_and_removed_rows() {
        let before = artifact(".", row(true, 10, 0, "."), finding('a'));
        let mut after = artifact(".", row(true, 10, 0, "."), finding('a'));
        after.rows[0].scope.file = Some(String::from("tests/replacement.rs"));

        let comparison = compare_artifacts(&before, &after).expect("comparison");

        assert!(comparison.matched.is_empty());
        assert!(comparison.changed.is_empty());
        assert_eq!(comparison.removed[0].file.as_deref(), Some("tests/api.rs"));
        assert_eq!(
            comparison.added[0].file.as_deref(),
            Some("tests/replacement.rs")
        );
    }

    #[test]
    fn artifact_compare_rejects_incomplete_and_incompatible_artifacts() {
        let complete = artifact(".", row(true, 1, 0, "."), finding('a'));
        let mut incomplete = complete.clone();
        incomplete.completion = RunCompletion {
            scope: CompletionScope::Repository,
            state: CompletionState::Incomplete,
            expected_targets: 1,
            detected_targets: 0,
            completed_targets: 0,
            skipped_targets: 1,
            issues: vec![CompletionIssue {
                language: Language::Rust,
                configured_root: String::from("."),
                stage: CompletionStage::Detection,
                message: String::from("not detected"),
            }],
        };
        assert!(matches!(
            compare_artifacts(&incomplete, &complete),
            Err(ArtifactComparisonError::IncompleteArtifact { side: "before" })
        ));

        let mut incompatible = complete.clone();
        incompatible.schema_version = String::from("0.2.0");
        assert!(matches!(
            compare_artifacts(&complete, &incompatible),
            Err(ArtifactComparisonError::IncompatibleSchema { side: "after", .. })
        ));
    }

    fn artifact(root: &str, row: SignalRow, findings: Findings) -> RunArtifact {
        RunArtifact {
            schema_version: String::from(ayni_core::AYNI_SIGNAL_SCHEMA_VERSION),
            metadata: RunArtifactMetadata {
                generated_at: format!("timestamp-{root}"),
                ayni_version: String::from("test"),
                invocation: InvocationContext {
                    command: String::from("analyze"),
                    languages: vec![Language::Rust],
                    scope: None,
                },
                output: OutputContext {
                    format: String::from(if root == "/a" { "json" } else { "md" }),
                    destination: root.to_string(),
                },
                config_path: format!("{root}/.ayni.toml"),
                repository_root: root.to_string(),
            },
            completion: RunCompletion::complete(CompletionScope::Repository, 1),
            rows: vec![row],
            findings: vec![findings],
        }
    }

    fn row(pass: bool, passed: u64, failed: u64, workspace_root: &str) -> SignalRow {
        SignalRow {
            kind: SignalKind::Test,
            language: Language::Rust,
            scope: Scope {
                workspace_root: workspace_root.to_string(),
                path: Some(String::from("crates/api")),
                package: Some(String::from("api")),
                file: Some(String::from("tests/api.rs")),
            },
            pass,
            result: SignalResult::Test(TestResult {
                total_tests: 10,
                passed,
                failed,
                duration_ms: Some(100),
                runner: String::from("cargo test"),
                failure: None,
            }),
            budget: Budget::Test(serde_json::json!({})),
            offenders: Offenders::Test(vec![offender()]),
        }
    }

    fn finding(character: char) -> Findings {
        Findings::Test(vec![Finding {
            metadata: FindingMetadata {
                id: id(character),
                verification: VerificationMetadata {
                    target: None,
                    command: Some(String::from(
                        "ayni verify test --language rust --file 'tests/api.rs'",
                    )),
                },
            },
            offender: offender(),
        }])
    }

    fn offender() -> TestFailure {
        TestFailure {
            file: Some(String::from("tests/api.rs")),
            line: Some(10),
            message: String::from("failed"),
            test_name: Some(String::from("api_test")),
        }
    }

    fn id(character: char) -> String {
        format!(
            "ayni:finding:v1:sha256:{}",
            character.to_string().repeat(64)
        )
    }
}
