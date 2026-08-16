use super::{AnalyzePlanning, AnalyzeTarget};
use ayni_core::{
    CompletionIssue, CompletionScope, CompletionStage, CompletionState, Language, RunCompletion,
    SignalKind, SignalRow,
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct ExpectedRowKey {
    language: Language,
    configured_root: String,
    kind: SignalKind,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct TargetKey {
    language: Language,
    configured_root: String,
}

impl TargetKey {
    fn from_target(target: &AnalyzeTarget) -> Self {
        Self {
            language: target.language,
            configured_root: normalize_configured_root(Some(&target.root)),
        }
    }
}

/// Reconcile collection evidence with the exact row set planned for runnable
/// targets. Rows outside that set and repeated rows are not serialized: their
/// evidence is retained as a collection-stage completion issue instead.
pub(crate) fn reconcile(
    planning: &AnalyzePlanning,
    scope: CompletionScope,
    requested_kind: Option<SignalKind>,
    emitted_rows: Vec<SignalRow>,
) -> (RunCompletion, Vec<SignalRow>) {
    let expected = expected_keys(planning, requested_kind);
    let runnable = planning
        .targets
        .iter()
        .map(TargetKey::from_target)
        .collect::<BTreeSet<_>>();
    let mut counts = BTreeMap::<ExpectedRowKey, usize>::new();
    let mut unexpected = Vec::new();
    let mut rows = Vec::new();

    for row in emitted_rows {
        let key = ExpectedRowKey {
            language: row.language,
            configured_root: normalize_configured_root(row.scope.path.as_deref()),
            kind: row.kind,
        };
        if expected.contains(&key) {
            let count = counts.entry(key).or_default();
            *count += 1;
            if *count == 1 {
                rows.push(row);
            }
        } else {
            unexpected.push(key);
        }
    }

    let mut failures = BTreeMap::<TargetKey, Vec<String>>::new();
    for key in &expected {
        match counts.get(key).copied().unwrap_or(0) {
            0 => failures
                .entry(TargetKey {
                    language: key.language,
                    configured_root: key.configured_root.clone(),
                })
                .or_default()
                .push(format!("missing expected {} row", kind_slug(key.kind))),
            count if count > 1 => failures
                .entry(TargetKey {
                    language: key.language,
                    configured_root: key.configured_root.clone(),
                })
                .or_default()
                .push(format!("emitted {count} {} rows", kind_slug(key.kind))),
            _ => {}
        }
    }
    for key in unexpected {
        let target = TargetKey {
            language: key.language,
            configured_root: key.configured_root.clone(),
        };
        let responsible = if runnable.contains(&target) {
            Some(target)
        } else {
            // An adapter may corrupt both language and scope. Attribute that
            // otherwise unowned evidence deterministically to the first
            // runnable target so completion still fails closed.
            runnable.first().cloned()
        };
        if let Some(target) = responsible {
            failures.entry(target).or_default().push(format!(
                "emitted unexpected {} row for {}:{}",
                kind_slug(key.kind),
                key.language,
                key.configured_root
            ));
        }
    }

    let mut issues = planning.issues.clone();
    for (target, reasons) in failures {
        issues.push(CompletionIssue {
            language: target.language,
            configured_root: target.configured_root,
            stage: CompletionStage::Collection,
            message: reasons.join("; "),
        });
    }
    issues.sort_by_key(|issue| {
        (
            issue.language,
            normalize_configured_root(Some(&issue.configured_root)),
            stage_rank(issue.stage),
        )
    });

    let skipped_targets = issues.len() as u64;
    let completed_targets = planning.expected_targets.saturating_sub(skipped_targets);
    (
        RunCompletion {
            scope,
            state: if skipped_targets == 0 {
                CompletionState::Complete
            } else {
                CompletionState::Incomplete
            },
            expected_targets: planning.expected_targets,
            detected_targets: planning.detected_targets,
            completed_targets,
            skipped_targets,
            issues,
        },
        rows,
    )
}

fn expected_keys(
    planning: &AnalyzePlanning,
    requested_kind: Option<SignalKind>,
) -> BTreeSet<ExpectedRowKey> {
    planning
        .targets
        .iter()
        .flat_map(|target| {
            let kinds = requested_kind.map_or_else(
                || super::enabled_signal_kinds(&target.run_context.policy),
                |kind| vec![kind],
            );
            kinds.into_iter().map(move |kind| ExpectedRowKey {
                language: target.language,
                configured_root: normalize_configured_root(Some(&target.root)),
                kind,
            })
        })
        .collect()
}

fn normalize_configured_root(root: Option<&str>) -> String {
    let normalized = root.unwrap_or(".").trim().replace('\\', "/");
    let normalized = normalized.trim_end_matches('/');
    if normalized.is_empty() || normalized == "." {
        String::from(".")
    } else {
        normalized.to_string()
    }
}

fn kind_slug(kind: SignalKind) -> &'static str {
    super::signal_kind_slug(kind)
}

fn stage_rank(stage: CompletionStage) -> u8 {
    match stage {
        CompletionStage::Detection => 0,
        CompletionStage::Resolution => 1,
        CompletionStage::Selection => 2,
        CompletionStage::Scheduling => 3,
        CompletionStage::Collection => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ayni_core::{AyniPolicy, ExecutionResolution, RunContext, Scope};
    use std::path::PathBuf;

    #[test]
    fn missing_expected_row_creates_collection_issue() {
        let root = PathBuf::from(".");
        let planning = AnalyzePlanning {
            targets: vec![AnalyzeTarget {
                language: Language::Rust,
                root: String::from("."),
                run_context: RunContext {
                    repo_root: root.clone(),
                    target_root: root.clone(),
                    workdir: root.clone(),
                    policy: AyniPolicy::default(),
                    scope: Scope::default(),
                    execution: ExecutionResolution::direct("cargo", root, "test", 100),
                    debug: false,
                },
            }],
            expected_targets: 1,
            detected_targets: 1,
            issues: Vec::new(),
        };

        let (completion, rows) = reconcile(
            &planning,
            CompletionScope::Requested,
            Some(SignalKind::Test),
            Vec::new(),
        );

        assert!(rows.is_empty());
        assert_eq!(completion.state, CompletionState::Incomplete);
        assert_eq!(completion.completed_targets, 0);
        assert_eq!(completion.skipped_targets, 1);
        assert_eq!(completion.issues[0].stage, CompletionStage::Collection);
        assert_eq!(completion.issues[0].configured_root, ".");
        assert!(
            completion.issues[0]
                .message
                .contains("missing expected test row")
        );
    }
}
