use crate::{CompletionState, RunArtifact};

/// Product-level outcome shared by full, focused, and impact execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunOutcome {
    Passed,
    QualityFailed,
    ExecutionIncomplete,
}

impl RunArtifact {
    /// Derive the command outcome from validated completion and typed rows.
    #[must_use]
    pub fn outcome(&self) -> RunOutcome {
        if self.completion.state == CompletionState::Incomplete
            || self
                .rows
                .iter()
                .any(|row| row.result.command_failure().is_some())
        {
            RunOutcome::ExecutionIncomplete
        } else if self.rows.iter().any(|row| !row.pass) {
            RunOutcome::QualityFailed
        } else {
            RunOutcome::Passed
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Budget, CompletionScope, Language, Offenders, RunArtifactMetadata, RunCompletion, Scope,
        SignalKind, SignalResult, SignalRow, TestResult,
    };

    fn artifact(state: CompletionState, pass: bool) -> RunArtifact {
        RunArtifact {
            schema_version: crate::AYNI_SIGNAL_SCHEMA_VERSION.into(),
            metadata: RunArtifactMetadata::default(),
            completion: RunCompletion {
                scope: CompletionScope::Repository,
                state,
                expected_targets: 1,
                detected_targets: 1,
                completed_targets: u64::from(state == CompletionState::Complete),
                skipped_targets: u64::from(state == CompletionState::Incomplete),
                issues: vec![],
            },
            findings: vec![],
            rows: vec![SignalRow {
                kind: SignalKind::Test,
                language: Language::Rust,
                scope: Scope::default(),
                pass,
                result: SignalResult::Test(TestResult {
                    total_tests: 1,
                    passed: u64::from(pass),
                    failed: u64::from(!pass),
                    duration_ms: None,
                    runner: "test".into(),
                    failure: None,
                }),
                budget: Budget::Test(crate::TestBudget::default()),
                offenders: Offenders::Test(vec![]),
            }],
        }
    }

    #[test]
    fn completion_precedes_quality_when_deriving_outcome() {
        assert_eq!(
            artifact(CompletionState::Incomplete, false).outcome(),
            RunOutcome::ExecutionIncomplete
        );
        assert_eq!(
            artifact(CompletionState::Complete, false).outcome(),
            RunOutcome::QualityFailed
        );
        assert_eq!(
            artifact(CompletionState::Complete, true).outcome(),
            RunOutcome::Passed
        );
    }
}
