use crate::language::Language;
use crate::runtime::Scope;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Semantic version of the JSON `RunArtifact` contract (`schema_version` field).
pub const AYNI_SIGNAL_SCHEMA_VERSION: &str = "0.4.0";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum SignalKind {
    Test,
    Coverage,
    Size,
    Complexity,
    Deps,
    Mutation,
}

/// Offender severity. Ordered so that `Warn < Fail`, which lets consumers sort
/// offenders by severity without ad-hoc rank helpers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Warn,
    Fail,
}

/// Runtime path that produced an artifact. Managed and host evidence are never
/// provenance-compatible, even when their normalized signal rows happen to match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    Managed,
    #[default]
    Host,
}

/// One exact runtime or analysis-tool version that participated in managed execution.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactToolVersion {
    pub tool: String,
    pub version: String,
}

/// Serializable inputs supplied by the orchestration layer when building an artifact.
/// Core deliberately does not read the clock, environment, or filesystem for these values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunArtifactMetadata {
    pub generated_at: String,
    pub ayni_version: String,
    pub invocation: InvocationContext,
    pub output: OutputContext,
    pub config_path: String,
    pub repository_root: String,
    pub execution_mode: ExecutionMode,
    pub contract_digest: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub environment_lock_fingerprint: Option<String>,
    pub source_fingerprint: String,
    #[serde(default)]
    pub tool_versions: Vec<ArtifactToolVersion>,
}

impl Default for RunArtifactMetadata {
    fn default() -> Self {
        let empty_digest = format!("sha256:{}", "0".repeat(64));
        Self {
            generated_at: String::new(),
            ayni_version: String::new(),
            invocation: InvocationContext::default(),
            output: OutputContext::default(),
            config_path: String::new(),
            repository_root: String::new(),
            execution_mode: ExecutionMode::Host,
            contract_digest: empty_digest.clone(),
            environment_lock_fingerprint: None,
            source_fingerprint: empty_digest,
            tool_versions: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct InvocationContext {
    pub command: String,
    #[serde(default)]
    pub languages: Vec<Language>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<Scope>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct OutputContext {
    pub format: String,
    pub destination: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AggregateStatus {
    Pass,
    Fail,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AggregateSummary {
    pub status: AggregateStatus,
    pub total_rows: u64,
    pub passing_rows: u64,
    pub failing_rows: u64,
    pub warning_offenders: u64,
    pub failing_offenders: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppliedThreshold {
    pub kind: SignalKind,
    pub language: Language,
    pub scope: Scope,
    pub budget: Budget,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OffenderSummary {
    pub kind: SignalKind,
    pub language: Language,
    pub scope: Scope,
    pub total: u64,
    pub warning_count: u64,
    pub failing_count: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FailureSummary {
    pub kind: SignalKind,
    pub language: Language,
    pub scope: Scope,
    pub category: String,
    pub classification: String,
    pub command: String,
    pub cwd: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompletionScope {
    Repository,
    Requested,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompletionState {
    Complete,
    Incomplete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompletionStage {
    Detection,
    Resolution,
    Selection,
    Scheduling,
    Collection,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompletionIssue {
    pub language: Language,
    pub configured_root: String,
    pub stage: CompletionStage,
    pub message: String,
}

/// Completion accounting for the exact target set represented by a run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunCompletion {
    pub scope: CompletionScope,
    pub state: CompletionState,
    pub expected_targets: u64,
    pub detected_targets: u64,
    pub completed_targets: u64,
    pub skipped_targets: u64,
    pub issues: Vec<CompletionIssue>,
}

impl RunCompletion {
    #[must_use]
    pub fn complete(scope: CompletionScope, target_count: u64) -> Self {
        Self {
            scope,
            state: CompletionState::Complete,
            expected_targets: target_count,
            detected_targets: target_count,
            completed_targets: target_count,
            skipped_targets: 0,
            issues: Vec::new(),
        }
    }

    fn validate(&self) -> Result<(), &'static str> {
        if self.detected_targets > self.expected_targets
            || self.completed_targets > self.detected_targets
            || self.skipped_targets != self.expected_targets - self.completed_targets
        {
            return Err("artifact completion target counts do not reconcile");
        }

        let detection_issues = self
            .issues
            .iter()
            .filter(|issue| issue.stage == CompletionStage::Detection)
            .count() as u64;
        if detection_issues != self.expected_targets - self.detected_targets
            || self.issues.len() as u64 != self.skipped_targets
        {
            return Err("artifact completion issues do not reconcile with skipped targets");
        }

        match self.state {
            CompletionState::Complete if self.skipped_targets == 0 && self.issues.is_empty() => {
                Ok(())
            }
            CompletionState::Incomplete if self.skipped_targets > 0 && !self.issues.is_empty() => {
                Ok(())
            }
            CompletionState::Complete => {
                Err("complete artifact must have no skipped targets or completion issues")
            }
            CompletionState::Incomplete => {
                Err("incomplete artifact must have skipped targets and completion issues")
            }
        }
    }
}

impl Default for RunCompletion {
    fn default() -> Self {
        Self::complete(CompletionScope::Repository, 0)
    }
}

/// Schema-v4 artifact. Rows are canonical analysis results; completion separately
/// records whether every target emitted its complete requested row set.
#[derive(Debug, Clone, PartialEq)]
pub struct RunArtifact {
    pub schema_version: String,
    pub metadata: RunArtifactMetadata,
    pub completion: RunCompletion,
    pub rows: Vec<SignalRow>,
    /// CLI-materialized finding metadata used only for the serialized row wire
    /// representation. Collection continues to use the typed offender payload.
    pub findings: Vec<Findings>,
}

impl Default for RunArtifact {
    fn default() -> Self {
        Self {
            schema_version: String::from(AYNI_SIGNAL_SCHEMA_VERSION),
            metadata: RunArtifactMetadata::default(),
            completion: RunCompletion::default(),
            rows: Vec::new(),
            findings: Vec::new(),
        }
    }
}

impl RunArtifact {
    /// Construct a staged artifact after validating provenance, completion, and
    /// every row's closed signal payload. Finding commands are materialized by
    /// the CLI before the artifact crosses a serialization boundary.
    pub fn new(
        metadata: RunArtifactMetadata,
        completion: RunCompletion,
        rows: Vec<SignalRow>,
    ) -> Result<Self, String> {
        let artifact = Self {
            schema_version: String::from(AYNI_SIGNAL_SCHEMA_VERSION),
            metadata,
            completion,
            rows,
            findings: Vec::new(),
        };
        validate_staged_artifact(&artifact)?;
        Ok(artifact)
    }

    #[must_use]
    pub fn aggregate(&self) -> AggregateSummary {
        let total_rows = self.rows.len() as u64;
        let passing_rows = self.rows.iter().filter(|row| row.pass).count() as u64;
        let (warning_offenders, failing_offenders) = self
            .rows
            .iter()
            .map(offender_counts)
            .fold((0, 0), |(warnings, failures), (warn, fail)| {
                (warnings + warn, failures + fail)
            });
        AggregateSummary {
            status: if self.completion.state == CompletionState::Complete
                && (self.completion.expected_targets == 0 || total_rows > 0)
                && passing_rows == total_rows
            {
                AggregateStatus::Pass
            } else {
                AggregateStatus::Fail
            },
            total_rows,
            passing_rows,
            failing_rows: total_rows - passing_rows,
            warning_offenders,
            failing_offenders,
        }
    }

    #[must_use]
    pub fn applied_thresholds(&self) -> Vec<AppliedThreshold> {
        self.rows
            .iter()
            .map(|row| AppliedThreshold {
                kind: row.kind,
                language: row.language,
                scope: row.scope.clone(),
                budget: row.budget.clone(),
            })
            .collect()
    }

    #[must_use]
    pub fn offender_summaries(&self) -> Vec<OffenderSummary> {
        self.rows
            .iter()
            .filter_map(|row| {
                let (warning_count, failing_count) = offender_counts(row);
                let total = warning_count + failing_count;
                (total > 0).then(|| OffenderSummary {
                    kind: row.kind,
                    language: row.language,
                    scope: row.scope.clone(),
                    total,
                    warning_count,
                    failing_count,
                })
            })
            .collect()
    }

    #[must_use]
    pub fn failure_summaries(&self) -> Option<Vec<FailureSummary>> {
        let failures: Vec<_> = self
            .rows
            .iter()
            .filter_map(|row| {
                row.result.command_failure().map(|failure| FailureSummary {
                    kind: row.kind,
                    language: row.language,
                    scope: row.scope.clone(),
                    category: failure.category.clone(),
                    classification: failure.classification.clone(),
                    command: failure.command.clone(),
                    cwd: failure.cwd.clone(),
                    exit_code: failure.exit_code,
                    message: failure.message.clone(),
                })
            })
            .collect();
        (!failures.is_empty()).then_some(failures)
    }
}

impl Serialize for RunArtifact {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        validate_serializable_artifact(self).map_err(serde::ser::Error::custom)?;
        RunArtifactSerialization::from(self).serialize(serializer)
    }
}

#[derive(Serialize)]
struct RunArtifactSerialization<'a> {
    schema_version: &'a str,
    generated_at: &'a str,
    ayni_version: &'a str,
    invocation: &'a InvocationContext,
    output: &'a OutputContext,
    config_path: &'a str,
    repository_root: &'a str,
    execution_mode: ExecutionMode,
    contract_digest: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    environment_lock_fingerprint: Option<&'a str>,
    source_fingerprint: &'a str,
    tool_versions: &'a [ArtifactToolVersion],
    completion: &'a RunCompletion,
    aggregate: AggregateSummary,
    applied_thresholds: Vec<AppliedThreshold>,
    rows: Vec<SignalRowSerialization<'a>>,
    offender_summaries: Vec<OffenderSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    failure_summaries: Option<Vec<FailureSummary>>,
}

impl<'a> From<&'a RunArtifact> for RunArtifactSerialization<'a> {
    fn from(artifact: &'a RunArtifact) -> Self {
        Self {
            schema_version: &artifact.schema_version,
            generated_at: &artifact.metadata.generated_at,
            ayni_version: &artifact.metadata.ayni_version,
            invocation: &artifact.metadata.invocation,
            output: &artifact.metadata.output,
            config_path: &artifact.metadata.config_path,
            repository_root: &artifact.metadata.repository_root,
            execution_mode: artifact.metadata.execution_mode,
            contract_digest: &artifact.metadata.contract_digest,
            environment_lock_fingerprint: artifact.metadata.environment_lock_fingerprint.as_deref(),
            source_fingerprint: &artifact.metadata.source_fingerprint,
            tool_versions: &artifact.metadata.tool_versions,
            completion: &artifact.completion,
            aggregate: artifact.aggregate(),
            applied_thresholds: artifact.applied_thresholds(),
            rows: artifact
                .rows
                .iter()
                .enumerate()
                .map(|(index, row)| SignalRowSerialization::new(row, artifact.findings.get(index)))
                .collect(),
            offender_summaries: artifact.offender_summaries(),
            failure_summaries: artifact.failure_summaries(),
        }
    }
}

#[derive(Serialize)]
struct SignalRowSerialization<'a> {
    kind: SignalKind,
    language: Language,
    scope: &'a Scope,
    pass: bool,
    result: &'a SignalResult,
    budget: &'a Budget,
    offenders: SerializedOffenders<'a>,
}

impl<'a> SignalRowSerialization<'a> {
    fn new(row: &'a SignalRow, findings: Option<&'a Findings>) -> Self {
        Self {
            kind: row.kind,
            language: row.language,
            scope: &row.scope,
            pass: row.pass,
            result: &row.result,
            budget: &row.budget,
            offenders: SerializedOffenders {
                raw: &row.offenders,
                findings,
            },
        }
    }
}

impl<'a> From<(&'a SignalRow, Option<&'a Findings>)> for SignalRowSerialization<'a> {
    fn from((row, findings): (&'a SignalRow, Option<&'a Findings>)) -> Self {
        Self::new(row, findings)
    }
}

struct SerializedOffenders<'a> {
    raw: &'a Offenders,
    findings: Option<&'a Findings>,
}

impl Serialize for SerializedOffenders<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self.findings {
            Some(findings) => findings.serialize(serializer),
            None => self.raw.serialize(serializer),
        }
    }
}

#[derive(Deserialize)]
struct RunArtifactWire {
    schema_version: String,
    generated_at: String,
    ayni_version: String,
    invocation: InvocationContext,
    output: OutputContext,
    config_path: String,
    repository_root: String,
    execution_mode: ExecutionMode,
    contract_digest: String,
    environment_lock_fingerprint: Option<String>,
    source_fingerprint: String,
    #[serde(default)]
    tool_versions: Vec<ArtifactToolVersion>,
    completion: RunCompletion,
    aggregate: AggregateSummary,
    applied_thresholds: Vec<AppliedThreshold>,
    rows: Vec<serde_json::Value>,
    offender_summaries: Vec<OffenderSummary>,
    #[serde(default)]
    failure_summaries: Option<Vec<FailureSummary>>,
}

struct ParsedArtifactRows {
    rows: Vec<SignalRow>,
    findings: Vec<Findings>,
}

impl ParsedArtifactRows {
    fn parse(row_values: Vec<serde_json::Value>) -> Result<Self, String> {
        let mut rows = Vec::with_capacity(row_values.len());
        let mut parsed_findings = Vec::with_capacity(row_values.len());

        for row_value in row_values {
            let parsed = ParsedArtifactRow::parse(row_value)?;
            rows.push(parsed.row);
            parsed_findings.push(parsed.findings);
        }

        let findings = if parsed_findings.iter().all(Findings::is_empty) {
            Vec::new()
        } else {
            parsed_findings
        };
        Ok(Self { rows, findings })
    }
}

struct ParsedArtifactRow {
    row: SignalRow,
    findings: Findings,
}

impl ParsedArtifactRow {
    fn parse(row_value: serde_json::Value) -> Result<Self, String> {
        let offenders = row_value
            .get("offenders")
            .ok_or_else(|| String::from("artifact row is missing offenders"))?;
        offenders
            .get("items")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| String::from("artifact row offenders are invalid"))?;
        let findings = serde_json::from_value::<Findings>(offenders.clone())
            .map_err(|error| format!("artifact offenders must be schema-v4 findings: {error}"))?;
        let row =
            serde_json::from_value::<SignalRow>(row_value).map_err(|error| error.to_string())?;
        Ok(Self { row, findings })
    }
}

impl RunArtifactWire {
    fn into_artifact(self) -> Result<RunArtifact, String> {
        let parsed = ParsedArtifactRows::parse(self.rows)?;
        let artifact = RunArtifact {
            schema_version: self.schema_version,
            metadata: RunArtifactMetadata {
                generated_at: self.generated_at,
                ayni_version: self.ayni_version,
                invocation: self.invocation,
                output: self.output,
                config_path: self.config_path,
                repository_root: self.repository_root,
                execution_mode: self.execution_mode,
                contract_digest: self.contract_digest,
                environment_lock_fingerprint: self.environment_lock_fingerprint,
                source_fingerprint: self.source_fingerprint,
                tool_versions: self.tool_versions,
            },
            completion: self.completion,
            rows: parsed.rows,
            findings: parsed.findings,
        };
        validate_deserialized_artifact(
            &artifact,
            self.aggregate,
            self.applied_thresholds,
            self.offender_summaries,
            self.failure_summaries,
        )?;
        Ok(artifact)
    }
}

fn validate_deserialized_artifact(
    artifact: &RunArtifact,
    aggregate: AggregateSummary,
    applied_thresholds: Vec<AppliedThreshold>,
    offender_summaries: Vec<OffenderSummary>,
    failure_summaries: Option<Vec<FailureSummary>>,
) -> Result<(), String> {
    validate_serializable_artifact(artifact)?;
    if artifact.aggregate() != aggregate
        || artifact.applied_thresholds() != applied_thresholds
        || artifact.offender_summaries() != offender_summaries
        || artifact.failure_summaries() != failure_summaries
    {
        return Err(String::from("artifact summaries must match canonical rows"));
    }
    Ok(())
}

fn validate_staged_artifact(artifact: &RunArtifact) -> Result<(), String> {
    validate_artifact_schema(artifact)?;
    validate_artifact_provenance(&artifact.metadata)?;
    artifact.completion.validate().map_err(String::from)?;
    validate_row_completion_structure(artifact)
}

fn validate_serializable_artifact(artifact: &RunArtifact) -> Result<(), String> {
    validate_staged_artifact(artifact)?;
    validate_artifact_findings(artifact)
}

fn validate_artifact_schema(artifact: &RunArtifact) -> Result<(), String> {
    if artifact.schema_version != AYNI_SIGNAL_SCHEMA_VERSION {
        Err(format!(
            "unsupported artifact schema_version {}; expected {}",
            artifact.schema_version, AYNI_SIGNAL_SCHEMA_VERSION
        ))
    } else {
        Ok(())
    }
}

fn validate_artifact_findings(artifact: &RunArtifact) -> Result<(), String> {
    if artifact.findings.is_empty() {
        if artifact
            .rows
            .iter()
            .all(|row| offenders_are_empty(&row.offenders))
        {
            return Ok(());
        }
        return Err(String::from(
            "artifact rows with offenders require one finding collection per row",
        ));
    }
    if artifact.findings.len() != artifact.rows.len() {
        return Err(String::from(
            "artifact finding collections must align one-for-one with rows",
        ));
    }
    for (row, findings) in artifact.rows.iter().zip(&artifact.findings) {
        if findings.kind() != row.kind || !findings.matches_offenders(&row.offenders) {
            return Err(String::from(
                "artifact finding collection does not match its signal row",
            ));
        }
        findings
            .validate_wire()
            .map_err(|error| format!("artifact finding metadata is invalid: {error}"))?;
    }
    Ok(())
}

fn offenders_are_empty(offenders: &Offenders) -> bool {
    match offenders {
        Offenders::Test(items) => items.is_empty(),
        Offenders::Coverage(items) => items.is_empty(),
        Offenders::Size(items) => items.is_empty(),
        Offenders::Complexity(items) => items.is_empty(),
        Offenders::Deps(items) => items.is_empty(),
        Offenders::Mutation(items) => items.is_empty(),
    }
}

fn validate_artifact_provenance(metadata: &RunArtifactMetadata) -> Result<(), String> {
    if !is_sha256_fingerprint(&metadata.contract_digest) {
        return Err(String::from(
            "artifact contract_digest must be a SHA-256 fingerprint",
        ));
    }
    if !is_sha256_fingerprint(&metadata.source_fingerprint) {
        return Err(String::from(
            "artifact source_fingerprint must be a SHA-256 fingerprint",
        ));
    }
    match metadata.execution_mode {
        ExecutionMode::Managed => {
            let fingerprint = metadata
                .environment_lock_fingerprint
                .as_deref()
                .ok_or_else(|| {
                    String::from("managed artifact must include environment_lock_fingerprint")
                })?;
            if !is_sha256_fingerprint(fingerprint) {
                return Err(String::from(
                    "artifact environment_lock_fingerprint must be a SHA-256 fingerprint",
                ));
            }
            if metadata.tool_versions.is_empty() {
                return Err(String::from(
                    "managed artifact must include locked tool versions",
                ));
            }
        }
        ExecutionMode::Host if metadata.environment_lock_fingerprint.is_some() => {
            return Err(String::from(
                "host artifact must not claim an environment lock fingerprint",
            ));
        }
        ExecutionMode::Host => {}
    }

    let mut previous = None;
    for tool in &metadata.tool_versions {
        if tool.tool.trim().is_empty() || tool.version.trim().is_empty() {
            return Err(String::from("artifact tool versions must be non-empty"));
        }
        if previous
            .as_ref()
            .is_some_and(|name: &&String| *name >= &tool.tool)
        {
            return Err(String::from(
                "artifact tool versions must be sorted by unique tool name",
            ));
        }
        previous = Some(&tool.tool);
    }
    Ok(())
}

fn is_sha256_fingerprint(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|digest| {
        digest.len() == 64
            && digest
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    })
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct CompletionRowKey {
    language: Language,
    configured_root: String,
    kind: SignalKind,
}

fn validate_row_completion_structure(artifact: &RunArtifact) -> Result<(), String> {
    validate_complete_rows_present(artifact)?;
    let mut keys = BTreeSet::new();
    let mut represented = BTreeMap::<(Language, String), BTreeSet<SignalKind>>::new();
    for row in &artifact.rows {
        row.validate_payloads()?;
        let configured_root = normalize_row_root(row.scope.path.as_deref());
        let key = CompletionRowKey {
            language: row.language,
            configured_root: configured_root.clone(),
            kind: row.kind,
        };
        if !keys.insert(key) {
            return Err(String::from("artifact row completion keys must be unique"));
        }
        represented
            .entry((row.language, configured_root))
            .or_default()
            .insert(row.kind);
    }

    validate_represented_target_count(artifact, represented.len() as u64)?;
    validate_consistent_repository_kind_sets(artifact, &represented)
}

fn validate_complete_rows_present(artifact: &RunArtifact) -> Result<(), String> {
    if artifact.completion.state == CompletionState::Complete
        && artifact.completion.expected_targets > 0
        && artifact.rows.is_empty()
    {
        Err(String::from(
            "complete artifact with expected targets must contain rows",
        ))
    } else {
        Ok(())
    }
}

fn validate_represented_target_count(
    artifact: &RunArtifact,
    represented_targets: u64,
) -> Result<(), String> {
    if represented_targets < artifact.completion.completed_targets {
        return Err(String::from(
            "artifact rows represent fewer targets than completion reports",
        ));
    }
    if artifact.completion.state == CompletionState::Complete
        && represented_targets != artifact.completion.completed_targets
    {
        return Err(String::from(
            "complete artifact represented targets do not reconcile with completion",
        ));
    }
    Ok(())
}

fn validate_consistent_repository_kind_sets(
    artifact: &RunArtifact,
    represented: &BTreeMap<(Language, String), BTreeSet<SignalKind>>,
) -> Result<(), String> {
    if artifact.completion.state != CompletionState::Complete
        || artifact.completion.scope != CompletionScope::Repository
    {
        return Ok(());
    }
    let mut kind_sets = represented.values();
    if let Some(expected) = kind_sets.next()
        && (expected.is_empty() || kind_sets.any(|kinds| kinds != expected))
    {
        return Err(String::from(
            "completed repository targets must have a consistent non-empty signal-kind set",
        ));
    }
    Ok(())
}

fn normalize_row_root(root: Option<&str>) -> String {
    let normalized = root.unwrap_or(".").trim().replace('\\', "/");
    let normalized = normalized.trim_end_matches('/');
    if normalized.is_empty() || normalized == "." {
        String::from(".")
    } else {
        normalized.to_string()
    }
}

impl<'de> Deserialize<'de> for RunArtifact {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        RunArtifactWire::deserialize(deserializer)?
            .into_artifact()
            .map_err(serde::de::Error::custom)
    }
}

impl SignalResult {
    #[must_use]
    pub fn command_failure(&self) -> Option<&CommandFailure> {
        match self {
            SignalResult::Test(value) => value.failure.as_ref(),
            SignalResult::Coverage(value) => value.failure.as_ref(),
            SignalResult::Size(value) => value.failure.as_ref(),
            SignalResult::Complexity(value) => value.failure.as_ref(),
            SignalResult::Deps(value) => value.failure.as_ref(),
            SignalResult::Mutation(value) => value.failure.as_ref(),
        }
    }
}

fn offender_counts(row: &SignalRow) -> (u64, u64) {
    match &row.offenders {
        Offenders::Test(items) => (0, items.len() as u64),
        Offenders::Coverage(items) => level_counts(items.iter().map(|item| item.level)),
        Offenders::Size(items) => level_counts(items.iter().map(|item| item.level)),
        Offenders::Complexity(items) => level_counts(items.iter().map(|item| item.level)),
        Offenders::Deps(items) => level_counts(items.iter().map(|item| item.level)),
        Offenders::Mutation(items) => level_counts(items.iter().map(|item| item.level)),
    }
}

fn level_counts(levels: impl Iterator<Item = Level>) -> (u64, u64) {
    levels.fold((0, 0), |(warnings, failures), level| match level {
        Level::Warn => (warnings + 1, failures),
        Level::Fail => (warnings, failures + 1),
    })
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SignalRow {
    pub kind: SignalKind,
    pub language: Language,
    pub scope: Scope,
    pub pass: bool,
    pub result: SignalResult,
    pub budget: Budget,
    pub offenders: Offenders,
}

impl SignalRow {
    /// Enforce the closed, per-signal payload union at every artifact boundary.
    pub fn validate_payloads(&self) -> Result<(), String> {
        crate::signal_validation::validate_signal_row(self)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SignalResult {
    Test(TestResult),
    Coverage(CoverageResult),
    Size(SizeResult),
    Complexity(ComplexityResult),
    Deps(DepsResult),
    Mutation(MutationResult),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommandFailure {
    pub category: String,
    pub classification: String,
    pub command: String,
    pub cwd: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Budget {
    Test(TestBudget),
    Coverage(CoverageBudget),
    Size(SizeBudget),
    Complexity(ComplexityBudget),
    Deps(DepsBudget),
    Mutation(MutationBudget),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct TestBudget {}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct CoverageBudget {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_percent_warn: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_percent_fail: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch_percent_warn: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch_percent_fail: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct SizeBudget {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<SizeBudgetRule>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warn: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fail: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SizeBudgetRule {
    pub glob: String,
    pub warn: u64,
    pub fail: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ComplexityBudget {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fn_cyclomatic: Option<FloatThresholdBudget>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fn_cognitive: Option<FloatThresholdBudget>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FloatThresholdBudget {
    pub warn: f64,
    pub fail: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct DepsBudget {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub forbidden: Option<BTreeMap<String, Vec<String>>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct MutationBudget {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "items", rename_all = "snake_case")]
pub enum Offenders {
    Test(Vec<TestFailure>),
    Coverage(Vec<CoverageOffender>),
    Size(Vec<SizeOffender>),
    Complexity(Vec<ComplexityOffender>),
    Deps(Vec<DepsOffender>),
    Mutation(Vec<MutationOffender>),
}

pub use crate::finding::{
    Finding, FindingError, FindingMetadata, Findings, OffenderIdentity, VerificationMetadata,
    VerificationTarget,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TestResult {
    pub total_tests: u64,
    pub passed: u64,
    pub failed: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    pub runner: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure: Option<CommandFailure>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TestFailure {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u64>,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub test_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoverageResult {
    /// Primary headline coverage percentage (0–100), comparable across languages.
    /// Adapters SHOULD set this to their single best metric when available (often line or
    /// statement coverage); consumers SHOULD fall back to [`Self::line_percent`] then
    /// [`Self::branch_percent`] when this is absent (for example legacy artifacts).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub percent: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_percent: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch_percent: Option<f64>,
    pub engine: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure: Option<CommandFailure>,
}

impl CoverageResult {
    #[must_use]
    pub fn headline_percent(&self) -> Option<f64> {
        self.percent.or(self.line_percent).or(self.branch_percent)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoverageOffender {
    pub file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u64>,
    pub value: f64,
    pub level: Level,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SizeResult {
    pub max_lines: u64,
    pub total_files: u64,
    pub warn_count: u64,
    pub fail_count: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure: Option<CommandFailure>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SizeOffender {
    pub file: String,
    pub value: u64,
    pub warn: u64,
    pub fail: u64,
    pub level: Level,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComplexityResult {
    pub engine: String,
    pub method: String,
    pub measured_functions: u64,
    pub max_fn_cyclomatic: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_fn_cognitive: Option<f64>,
    pub warn_count: u64,
    pub fail_count: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure: Option<CommandFailure>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComplexityOffender {
    pub file: String,
    pub line: u64,
    pub function: String,
    pub cyclomatic: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cognitive: Option<f64>,
    pub level: Level,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DepsResult {
    pub crate_count: u64,
    pub edge_count: u64,
    pub violation_count: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure: Option<CommandFailure>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DepsOffender {
    pub from: String,
    pub to: String,
    pub rule: String,
    pub level: Level,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MutationResult {
    pub engine: String,
    pub killed: u64,
    pub survived: u64,
    pub timeout: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure: Option<CommandFailure>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MutationOffender {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u64>,
    pub mutation_kind: String,
    pub message: String,
    pub level: Level,
}

#[cfg(test)]
#[path = "signal_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "signal_coverage_tests.rs"]
mod coverage_result_tests;
