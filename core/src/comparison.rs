use crate::{
    AYNI_SIGNAL_SCHEMA_VERSION, Budget, CompletionState, Language, Offenders, RunArtifact,
    SignalKind, SignalResult, SignalRow,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Version of the deterministic two-artifact comparison document.
pub const ARTIFACT_COMPARISON_SCHEMA_VERSION: &str = "0.1.0";

/// The stable row identity used when comparing complete schema-v4 artifacts.
/// Checkout-specific workspace roots are deliberately excluded.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SignalRowKey {
    pub kind: SignalKind,
    pub language: Language,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
}

impl From<&SignalRow> for SignalRowKey {
    fn from(row: &SignalRow) -> Self {
        Self {
            kind: row.kind,
            language: row.language,
            path: row.scope.path.clone(),
            package: row.scope.package.clone(),
            file: row.scope.file.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum MetricValue {
    Integer(u64),
    Float(f64),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValueChange<T> {
    pub before: T,
    pub after: T,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetricChange {
    pub metric: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<MetricValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<MetricValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FindingIdChanges {
    pub added: Vec<String>,
    pub removed: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct RowChangeSet {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pass: Option<ValueChange<bool>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub metrics: Vec<MetricChange>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finding_ids: Option<FindingIdChanges>,
}

impl RowChangeSet {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.pass.is_none() && self.metrics.is_empty() && self.finding_ids.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MatchedRowComparison {
    pub key: SignalRowKey,
    pub changed: bool,
    pub changes: RowChangeSet,
}

/// Deterministically ordered comparison. `matched` contains every row present
/// in both artifacts; its `changed` flag distinguishes changed and unchanged
/// matches without duplicating rows in another collection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArtifactComparison {
    pub comparison_schema_version: String,
    pub artifact_schema_version: String,
    pub matched: Vec<MatchedRowComparison>,
    pub changed: Vec<SignalRowKey>,
    pub added: Vec<SignalRowKey>,
    pub removed: Vec<SignalRowKey>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArtifactComparisonError {
    IncompatibleSchema {
        side: &'static str,
        found: String,
    },
    IncompleteArtifact {
        side: &'static str,
    },
    IncompatibleProvenance {
        field: &'static str,
        before: String,
        after: String,
    },
    UncomparableProvenance {
        side: &'static str,
        reason: String,
    },
    InvalidArtifact {
        side: &'static str,
        reason: String,
    },
}

impl std::fmt::Display for ArtifactComparisonError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::IncompatibleSchema { side, found } => write!(
                formatter,
                "{side} artifact uses incompatible schema {found}; expected {AYNI_SIGNAL_SCHEMA_VERSION}"
            ),
            Self::IncompleteArtifact { side } => {
                write!(formatter, "{side} artifact is incomplete")
            }
            Self::IncompatibleProvenance {
                field,
                before,
                after,
            } => write!(
                formatter,
                "artifact provenance is incompatible for {field}: before={before}, after={after}"
            ),
            Self::UncomparableProvenance { side, reason } => {
                write!(formatter, "{side} artifact cannot be compared: {reason}")
            }
            Self::InvalidArtifact { side, reason } => {
                write!(formatter, "{side} artifact is invalid: {reason}")
            }
        }
    }
}

impl std::error::Error for ArtifactComparisonError {}

/// Compare two validated, complete schema-v4 artifacts. Metadata outside row
/// identity and row measurements (including roots, timestamps, output mode and
/// any repository state) cannot affect this result.
pub fn compare_artifacts(
    before: &RunArtifact,
    after: &RunArtifact,
) -> Result<ArtifactComparison, ArtifactComparisonError> {
    let before_rows = validate_and_index(before, "before")?;
    let after_rows = validate_and_index(after, "after")?;
    validate_compatible_provenance(before, after)?;
    let mut matched = Vec::new();
    let mut changed = Vec::new();
    let mut added = Vec::new();
    let mut removed = Vec::new();

    for (key, before_row) in &before_rows {
        if let Some(after_row) = after_rows.get(key) {
            let changes = compare_rows(before, before_row, after, after_row)?;
            if !changes.is_empty() {
                changed.push(key.clone());
            }
            matched.push(MatchedRowComparison {
                key: key.clone(),
                changed: !changes.is_empty(),
                changes,
            });
        } else {
            removed.push(key.clone());
        }
    }
    for key in after_rows.keys() {
        if !before_rows.contains_key(key) {
            added.push(key.clone());
        }
    }

    Ok(ArtifactComparison {
        comparison_schema_version: String::from(ARTIFACT_COMPARISON_SCHEMA_VERSION),
        artifact_schema_version: String::from(AYNI_SIGNAL_SCHEMA_VERSION),
        matched,
        changed,
        added,
        removed,
    })
}

fn validate_compatible_provenance(
    before: &RunArtifact,
    after: &RunArtifact,
) -> Result<(), ArtifactComparisonError> {
    if before.metadata.execution_mode != after.metadata.execution_mode {
        return Err(ArtifactComparisonError::IncompatibleProvenance {
            field: "execution_mode",
            before: format!("{:?}", before.metadata.execution_mode),
            after: format!("{:?}", after.metadata.execution_mode),
        });
    }
    for (side, artifact) in [("before", before), ("after", after)] {
        if artifact.metadata.execution_mode == crate::ExecutionMode::Host {
            return Err(ArtifactComparisonError::UncomparableProvenance {
                side,
                reason: String::from(
                    "host execution does not lock tool versions; compare managed artifacts",
                ),
            });
        }
    }
    let pairs = [
        (
            "ayni_version",
            before.metadata.ayni_version.clone(),
            after.metadata.ayni_version.clone(),
        ),
        (
            "contract_digest",
            before.metadata.contract_digest.clone(),
            after.metadata.contract_digest.clone(),
        ),
        (
            "environment_lock_fingerprint",
            before
                .metadata
                .environment_lock_fingerprint
                .clone()
                .unwrap_or_else(|| String::from("none")),
            after
                .metadata
                .environment_lock_fingerprint
                .clone()
                .unwrap_or_else(|| String::from("none")),
        ),
        (
            "tool_versions",
            serde_json::to_string(&before.metadata.tool_versions).unwrap_or_default(),
            serde_json::to_string(&after.metadata.tool_versions).unwrap_or_default(),
        ),
    ];
    for (field, before, after) in pairs {
        if before != after {
            return Err(ArtifactComparisonError::IncompatibleProvenance {
                field,
                before,
                after,
            });
        }
    }
    Ok(())
}

fn validate_and_index<'a>(
    artifact: &'a RunArtifact,
    side: &'static str,
) -> Result<BTreeMap<SignalRowKey, (usize, &'a SignalRow)>, ArtifactComparisonError> {
    if artifact.schema_version != AYNI_SIGNAL_SCHEMA_VERSION {
        return Err(ArtifactComparisonError::IncompatibleSchema {
            side,
            found: artifact.schema_version.clone(),
        });
    }
    if artifact.completion.state != CompletionState::Complete {
        return Err(ArtifactComparisonError::IncompleteArtifact { side });
    }
    // Re-serialization exercises completion reconciliation and the current
    // schema's other fail-closed artifact invariants for manually built values.
    serde_json::to_value(artifact).map_err(|error| ArtifactComparisonError::InvalidArtifact {
        side,
        reason: error.to_string(),
    })?;

    if !artifact.findings.is_empty() && artifact.findings.len() != artifact.rows.len() {
        return invalid(side, "finding collections do not align with rows");
    }
    let mut indexed = BTreeMap::new();
    for (index, row) in artifact.rows.iter().enumerate() {
        validate_row(artifact, index, row, side)?;
        let key = SignalRowKey::from(row);
        if indexed.insert(key, (index, row)).is_some() {
            return invalid(side, "duplicate row matching key");
        }
    }
    Ok(indexed)
}

fn validate_row(
    artifact: &RunArtifact,
    index: usize,
    row: &SignalRow,
    side: &'static str,
) -> Result<(), ArtifactComparisonError> {
    let variants_match = matches!(
        (row.kind, &row.result, &row.budget, &row.offenders),
        (
            SignalKind::Test,
            SignalResult::Test(_),
            Budget::Test(_),
            Offenders::Test(_)
        ) | (
            SignalKind::Coverage,
            SignalResult::Coverage(_),
            Budget::Coverage(_),
            Offenders::Coverage(_)
        ) | (
            SignalKind::Size,
            SignalResult::Size(_),
            Budget::Size(_),
            Offenders::Size(_)
        ) | (
            SignalKind::Complexity,
            SignalResult::Complexity(_),
            Budget::Complexity(_),
            Offenders::Complexity(_)
        ) | (
            SignalKind::Deps,
            SignalResult::Deps(_),
            Budget::Deps(_),
            Offenders::Deps(_)
        ) | (
            SignalKind::Mutation,
            SignalResult::Mutation(_),
            Budget::Mutation(_),
            Offenders::Mutation(_)
        )
    );
    if !variants_match {
        return invalid(side, "row kind does not match its typed payloads");
    }
    for path in [
        row.scope.path.as_deref(),
        row.scope.package.as_deref(),
        row.scope.file.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        if is_absolute_like(path) {
            return invalid(side, "row matching scope must use relative paths");
        }
    }

    let offender_count = offender_count(&row.offenders);
    if let Some(findings) = artifact.findings.get(index) {
        if findings.kind() != row.kind
            || findings.ids().len() != offender_count
            || !findings.matches_offenders(&row.offenders)
        {
            return invalid(side, "finding collection does not match its row");
        }
        let ids = findings.ids();
        if ids.iter().collect::<BTreeSet<_>>().len() != ids.len() {
            return invalid(side, "row contains duplicate finding IDs");
        }
    } else if offender_count != 0 {
        return invalid(side, "row offenders are missing finding IDs");
    }
    metrics(&row.result)
        .map_err(|reason| ArtifactComparisonError::InvalidArtifact { side, reason })?;
    Ok(())
}

fn compare_rows(
    before_artifact: &RunArtifact,
    (before_index, before): &(usize, &SignalRow),
    after_artifact: &RunArtifact,
    (after_index, after): &(usize, &SignalRow),
) -> Result<RowChangeSet, ArtifactComparisonError> {
    let pass = (before.pass != after.pass).then_some(ValueChange {
        before: before.pass,
        after: after.pass,
    });
    let before_metrics =
        metrics(&before.result).map_err(|reason| ArtifactComparisonError::InvalidArtifact {
            side: "before",
            reason,
        })?;
    let after_metrics =
        metrics(&after.result).map_err(|reason| ArtifactComparisonError::InvalidArtifact {
            side: "after",
            reason,
        })?;
    let names = before_metrics
        .keys()
        .chain(after_metrics.keys())
        .copied()
        .collect::<BTreeSet<_>>();
    let metric_changes = names
        .into_iter()
        .filter_map(|name| {
            let before = before_metrics.get(name).cloned();
            let after = after_metrics.get(name).cloned();
            (before != after).then(|| MetricChange {
                metric: name.to_string(),
                before,
                after,
            })
        })
        .collect();

    let before_ids = finding_ids(before_artifact, *before_index);
    let after_ids = finding_ids(after_artifact, *after_index);
    let added = after_ids
        .difference(&before_ids)
        .cloned()
        .collect::<Vec<_>>();
    let removed = before_ids
        .difference(&after_ids)
        .cloned()
        .collect::<Vec<_>>();
    let finding_ids =
        (!added.is_empty() || !removed.is_empty()).then_some(FindingIdChanges { added, removed });
    Ok(RowChangeSet {
        pass,
        metrics: metric_changes,
        finding_ids,
    })
}

fn finding_ids(artifact: &RunArtifact, index: usize) -> BTreeSet<String> {
    artifact
        .findings
        .get(index)
        .map(|findings| findings.ids().into_iter().map(String::from).collect())
        .unwrap_or_default()
}

fn metrics(result: &SignalResult) -> Result<BTreeMap<&'static str, MetricValue>, String> {
    let mut values = BTreeMap::new();
    macro_rules! float {
        ($name:literal, $value:expr) => {{
            let value = $value;
            if !value.is_finite() {
                return Err(format!("metric {} is not finite", $name));
            }
            values.insert($name, MetricValue::Float(value));
        }};
    }
    match result {
        SignalResult::Test(value) => {
            values.insert("total_tests", MetricValue::Integer(value.total_tests));
            values.insert("passed", MetricValue::Integer(value.passed));
            values.insert("failed", MetricValue::Integer(value.failed));
            if let Some(duration) = value.duration_ms {
                values.insert("duration_ms", MetricValue::Integer(duration));
            }
        }
        SignalResult::Coverage(value) => {
            if let Some(value) = value.percent {
                float!("percent", value);
            }
            if let Some(value) = value.line_percent {
                float!("line_percent", value);
            }
            if let Some(value) = value.branch_percent {
                float!("branch_percent", value);
            }
        }
        SignalResult::Size(value) => {
            values.insert("max_lines", MetricValue::Integer(value.max_lines));
            values.insert("total_files", MetricValue::Integer(value.total_files));
            values.insert("warn_count", MetricValue::Integer(value.warn_count));
            values.insert("fail_count", MetricValue::Integer(value.fail_count));
        }
        SignalResult::Complexity(value) => {
            values.insert(
                "measured_functions",
                MetricValue::Integer(value.measured_functions),
            );
            float!("max_fn_cyclomatic", value.max_fn_cyclomatic);
            if let Some(value) = value.max_fn_cognitive {
                float!("max_fn_cognitive", value);
            }
            values.insert("warn_count", MetricValue::Integer(value.warn_count));
            values.insert("fail_count", MetricValue::Integer(value.fail_count));
        }
        SignalResult::Deps(value) => {
            values.insert("crate_count", MetricValue::Integer(value.crate_count));
            values.insert("edge_count", MetricValue::Integer(value.edge_count));
            values.insert(
                "violation_count",
                MetricValue::Integer(value.violation_count),
            );
        }
        SignalResult::Mutation(value) => {
            values.insert("killed", MetricValue::Integer(value.killed));
            values.insert("survived", MetricValue::Integer(value.survived));
            values.insert("timeout", MetricValue::Integer(value.timeout));
            if let Some(value) = value.score {
                float!("score", value);
            }
        }
    }
    Ok(values)
}

fn offender_count(offenders: &Offenders) -> usize {
    match offenders {
        Offenders::Test(items) => items.len(),
        Offenders::Coverage(items) => items.len(),
        Offenders::Size(items) => items.len(),
        Offenders::Complexity(items) => items.len(),
        Offenders::Deps(items) => items.len(),
        Offenders::Mutation(items) => items.len(),
    }
}

fn is_absolute_like(path: &str) -> bool {
    path.starts_with('/') || path.starts_with('\\') || path.as_bytes().get(1) == Some(&b':')
}

fn invalid<T>(side: &'static str, reason: &str) -> Result<T, ArtifactComparisonError> {
    Err(ArtifactComparisonError::InvalidArtifact {
        side,
        reason: reason.to_string(),
    })
}
