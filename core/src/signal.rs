use crate::language::Language;
use crate::runtime::Scope;
use serde::ser::SerializeStruct;
use serde::{Deserialize, Serialize};

/// Semantic version of the JSON `RunArtifact` contract (`schema_version` field).
pub const AYNI_SIGNAL_SCHEMA_VERSION: &str = "0.2.0";

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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Delta {
    #[serde(default)]
    pub changes: serde_json::Value,
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

/// Schema-v2 artifact. Rows are the sole canonical analysis result; all aggregate,
/// threshold, offender, and failure views are derived during serialization.
#[derive(Debug, Clone, PartialEq)]
pub struct RunArtifact {
    pub schema_version: String,
    pub metadata: RunArtifactMetadata,
    pub rows: Vec<SignalRow>,
}

impl Default for RunArtifact {
    fn default() -> Self {
        Self {
            schema_version: String::from(AYNI_SIGNAL_SCHEMA_VERSION),
            metadata: RunArtifactMetadata::default(),
            rows: Vec::new(),
        }
    }
}

impl RunArtifact {
    #[must_use]
    pub fn new(metadata: RunArtifactMetadata, rows: Vec<SignalRow>) -> Self {
        Self {
            schema_version: String::from(AYNI_SIGNAL_SCHEMA_VERSION),
            metadata,
            rows,
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
            status: if passing_rows == total_rows {
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
                command_failure(&row.result).map(|failure| FailureSummary {
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
        let failures = self.failure_summaries();
        let mut state =
            serializer.serialize_struct("RunArtifact", if failures.is_some() { 12 } else { 11 })?;
        state.serialize_field("schema_version", &self.schema_version)?;
        state.serialize_field("generated_at", &self.metadata.generated_at)?;
        state.serialize_field("ayni_version", &self.metadata.ayni_version)?;
        state.serialize_field("invocation", &self.metadata.invocation)?;
        state.serialize_field("output", &self.metadata.output)?;
        state.serialize_field("config_path", &self.metadata.config_path)?;
        state.serialize_field("repository_root", &self.metadata.repository_root)?;
        state.serialize_field("aggregate", &self.aggregate())?;
        state.serialize_field("applied_thresholds", &self.applied_thresholds())?;
        state.serialize_field("rows", &self.rows)?;
        state.serialize_field("offender_summaries", &self.offender_summaries())?;
        if let Some(failures) = failures {
            state.serialize_field("failure_summaries", &failures)?;
        }
        state.end()
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
    aggregate: AggregateSummary,
    applied_thresholds: Vec<AppliedThreshold>,
    rows: Vec<SignalRow>,
    offender_summaries: Vec<OffenderSummary>,
    #[serde(default)]
    failure_summaries: Option<Vec<FailureSummary>>,
}

impl<'de> Deserialize<'de> for RunArtifact {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = RunArtifactWire::deserialize(deserializer)?;
        let artifact = Self {
            schema_version: wire.schema_version,
            metadata: RunArtifactMetadata {
                generated_at: wire.generated_at,
                ayni_version: wire.ayni_version,
                invocation: wire.invocation,
                output: wire.output,
                config_path: wire.config_path,
                repository_root: wire.repository_root,
            },
            rows: wire.rows,
        };
        if artifact.aggregate() != wire.aggregate
            || artifact.applied_thresholds() != wire.applied_thresholds
            || artifact.offender_summaries() != wire.offender_summaries
            || artifact.failure_summaries() != wire.failure_summaries
        {
            return Err(serde::de::Error::custom(
                "artifact summaries must match canonical rows",
            ));
        }
        Ok(artifact)
    }
}

fn command_failure(result: &SignalResult) -> Option<&CommandFailure> {
    match result {
        SignalResult::Test(value) => value.failure.as_ref(),
        SignalResult::Coverage(value) => value.failure.as_ref(),
        SignalResult::Complexity(value) => value.failure.as_ref(),
        SignalResult::Mutation(value) => value.failure.as_ref(),
        SignalResult::Size(_) | SignalResult::Deps(_) => None,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delta_vs_previous: Option<Delta>,
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
mod run_artifact_tests {
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
                delta_vs_previous: None,
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
            }),
            budget: Budget::Size(serde_json::json!({ "warn": 10, "fail": 30 })),
            offenders: Offenders::Size(vec![SizeOffender {
                file: String::from("src/lib.rs"),
                value: 20,
                warn: 10,
                fail: 30,
                level: Level::Warn,
            }]),
            delta_vs_previous: None,
        };
        let artifact = RunArtifact::new(RunArtifactMetadata::default(), vec![row]);

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
