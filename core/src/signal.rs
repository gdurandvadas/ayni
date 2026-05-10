use crate::language::Language;
use crate::runtime::Scope;
use serde::{Deserialize, Serialize};

/// Semantic version of the JSON `RunArtifact` contract (`schema_version` field). Pre-1.0; bump when breaking.
pub const AYNI_SIGNAL_SCHEMA_VERSION: &str = "0.1.0";

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Passed,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunArtifact {
    pub schema_version: String,
    #[serde(default)]
    pub rows: Vec<SignalRow>,
}

impl Default for RunArtifact {
    fn default() -> Self {
        Self {
            schema_version: String::from(AYNI_SIGNAL_SCHEMA_VERSION),
            rows: Vec::new(),
        }
    }
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delta_vs_baseline: Option<Delta>,
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
