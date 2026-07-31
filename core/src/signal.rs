use crate::language::Language;
use crate::runtime::Scope;
use serde::{Deserialize, Serialize};

/// Semantic version of the JSON `RunArtifact` contract (`schema_version` field).
pub const AYNI_SIGNAL_SCHEMA_VERSION: &str = "0.3.0";

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

/// Serializable inputs supplied by the orchestration layer when building an artifact.
/// Core deliberately does not read the clock, environment, or filesystem for these values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RunArtifactMetadata {
    pub generated_at: String,
    pub ayni_version: String,
    pub invocation: InvocationContext,
    pub output: OutputContext,
    pub config_path: String,
    pub repository_root: String,
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

/// Schema-v3 artifact. Rows are canonical analysis results; completion separately
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
    #[must_use]
    pub fn new(
        metadata: RunArtifactMetadata,
        completion: RunCompletion,
        rows: Vec<SignalRow>,
    ) -> Self {
        Self {
            schema_version: String::from(AYNI_SIGNAL_SCHEMA_VERSION),
            metadata,
            completion,
            rows,
            findings: Vec::new(),
        }
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
        if self.schema_version != AYNI_SIGNAL_SCHEMA_VERSION {
            return Err(serde::ser::Error::custom(format!(
                "unsupported artifact schema_version {}; expected {}",
                self.schema_version, AYNI_SIGNAL_SCHEMA_VERSION
            )));
        }
        self.completion
            .validate()
            .map_err(serde::ser::Error::custom)?;
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
    completion: RunCompletion,
    aggregate: AggregateSummary,
    applied_thresholds: Vec<AppliedThreshold>,
    rows: Vec<serde_json::Value>,
    offender_summaries: Vec<OffenderSummary>,
    #[serde(default)]
    failure_summaries: Option<Vec<FailureSummary>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OffenderWireEncoding {
    Legacy,
    Finding,
}

struct ParsedArtifactRows {
    rows: Vec<SignalRow>,
    findings: Vec<Findings>,
}

impl ParsedArtifactRows {
    fn parse(row_values: Vec<serde_json::Value>) -> Result<Self, String> {
        let mut rows = Vec::with_capacity(row_values.len());
        let mut parsed_findings = Vec::with_capacity(row_values.len());
        let mut encoding = None;

        for row_value in row_values {
            let parsed = ParsedArtifactRow::parse(row_value, &mut encoding)?;
            rows.push(parsed.row);
            if let Some(findings) = parsed.findings {
                parsed_findings.push(findings);
            }
        }

        let findings = match encoding {
            Some(OffenderWireEncoding::Finding) => parsed_findings,
            Some(OffenderWireEncoding::Legacy) | None => Vec::new(),
        };
        Ok(Self { rows, findings })
    }
}

struct ParsedArtifactRow {
    row: SignalRow,
    findings: Option<Findings>,
}

impl ParsedArtifactRow {
    fn parse(
        row_value: serde_json::Value,
        observed_encoding: &mut Option<OffenderWireEncoding>,
    ) -> Result<Self, String> {
        let offenders = row_value
            .get("offenders")
            .ok_or_else(|| String::from("artifact row is missing offenders"))?;
        let items = offenders
            .get("items")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| String::from("artifact row offenders are invalid"))?;
        let encoding = if items
            .iter()
            .any(|item| item.get("id").is_some() || item.get("verification").is_some())
        {
            OffenderWireEncoding::Finding
        } else {
            OffenderWireEncoding::Legacy
        };

        if !items.is_empty() {
            match observed_encoding {
                Some(observed) if *observed != encoding => {
                    return Err(String::from(
                        "artifact rows mix finding and legacy offender encodings",
                    ));
                }
                Some(_) => {}
                None => *observed_encoding = Some(encoding),
            }
        }

        let findings = if encoding == OffenderWireEncoding::Finding || items.is_empty() {
            Some(
                serde_json::from_value::<Findings>(offenders.clone())
                    .map_err(|error| error.to_string())?,
            )
        } else {
            None
        };
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
    if artifact.schema_version != AYNI_SIGNAL_SCHEMA_VERSION {
        return Err(format!(
            "unsupported artifact schema_version {}; expected {}",
            artifact.schema_version, AYNI_SIGNAL_SCHEMA_VERSION
        ));
    }
    artifact.completion.validate().map_err(String::from)?;
    if artifact.aggregate() != aggregate
        || artifact.applied_thresholds() != applied_thresholds
        || artifact.offender_summaries() != offender_summaries
        || artifact.failure_summaries() != failure_summaries
    {
        return Err(String::from("artifact summaries must match canonical rows"));
    }
    Ok(())
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
    Test(serde_json::Value),
    Coverage(serde_json::Value),
    Size(serde_json::Value),
    Complexity(serde_json::Value),
    Deps(serde_json::Value),
    Mutation(serde_json::Value),
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
mod tests {
    use super::*;
    use crate::language::Language;
    use crate::runtime::Scope;

    #[test]
    fn run_artifact_json_roundtrip_preserves_rows() {
        let artifact = RunArtifact::new(
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
                budget: Budget::Test(serde_json::json!({})),
                offenders: Offenders::Test(vec![TestFailure {
                    file: Some(String::from("src/lib.rs")),
                    line: Some(42),
                    message: String::from("assertion failed"),
                    test_name: Some(String::from("does_thing")),
                }]),
            }],
        );

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
            budget: Budget::Size(serde_json::json!({ "warn": 10, "fail": 30 })),
            offenders: Offenders::Size(vec![SizeOffender {
                file: String::from("src/lib.rs"),
                value: 20,
                warn: 10,
                fail: 30,
                level: Level::Warn,
            }]),
        };
        let artifact = RunArtifact::new(
            RunArtifactMetadata::default(),
            RunCompletion::complete(CompletionScope::Repository, 1),
            vec![row],
        );

        assert_eq!(artifact.aggregate().status, AggregateStatus::Pass);
        assert_eq!(artifact.aggregate().warning_offenders, 1);
        assert_eq!(
            artifact.applied_thresholds()[0].budget,
            Budget::Size(serde_json::json!({ "warn": 10, "fail": 30 }))
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
                    budget: Budget::Size(serde_json::json!({})),
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
                    budget: Budget::Deps(serde_json::json!({})),
                    offenders: Offenders::Deps(Vec::new()),
                },
            ],
        );

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
        );

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
        );
        let mut value = serde_json::to_value(artifact).expect("serialize");
        value["completion"]["state"] = serde_json::json!("incomplete");

        let error = serde_json::from_value::<RunArtifact>(value).expect_err("invalid completion");
        assert!(error.to_string().contains("incomplete artifact"));
    }

    #[test]
    fn deserialization_rejects_historical_schema() {
        let artifact = RunArtifact::new(
            RunArtifactMetadata::default(),
            RunCompletion::complete(CompletionScope::Repository, 0),
            Vec::new(),
        );
        let mut value = serde_json::to_value(artifact).expect("serialize");
        value["schema_version"] = serde_json::json!("0.2.0");

        let error = serde_json::from_value::<RunArtifact>(value).expect_err("historical schema");
        assert!(
            error
                .to_string()
                .contains("unsupported artifact schema_version")
        );
    }
}

#[cfg(test)]
mod coverage_result_tests {
    use super::CoverageResult;

    #[test]
    fn headline_percent_prefers_percent_then_line_then_branch() {
        assert_eq!(
            CoverageResult {
                percent: Some(90.0),
                line_percent: Some(70.0),
                branch_percent: Some(60.0),
                engine: String::new(),
                status: String::new(),
                failure: None,
            }
            .headline_percent(),
            Some(90.0)
        );
        assert_eq!(
            CoverageResult {
                percent: None,
                line_percent: Some(71.5),
                branch_percent: Some(60.0),
                engine: String::new(),
                status: String::new(),
                failure: None,
            }
            .headline_percent(),
            Some(71.5)
        );
        assert_eq!(
            CoverageResult {
                percent: None,
                line_percent: None,
                branch_percent: Some(55.0),
                engine: String::new(),
                status: String::new(),
                failure: None,
            }
            .headline_percent(),
            Some(55.0)
        );
    }
}
