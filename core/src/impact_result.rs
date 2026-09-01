use crate::{Findings, ImpactPlan, RunOutcome, SelectedCheck, SignalRow};
use serde::{Deserialize, Serialize};

pub const IMPACT_SCHEMA_VERSION: &str = "0.1.0";

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ImpactExecutionState {
    Complete,
    Incomplete,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImpactExecutionIssue {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub check: Option<SelectedCheck>,
    pub message: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImpactExecution {
    pub state: ImpactExecutionState,
    pub planned_jobs: u64,
    pub completed_jobs: u64,
    pub skipped_jobs: u64,
    pub issues: Vec<ImpactExecutionIssue>,
}

#[derive(Debug, Serialize)]
pub struct RepositoryCompletionMarker {
    pub evaluated: bool,
    pub required_command: &'static str,
}

#[derive(Debug, Serialize)]
pub struct ImpactAggregate {
    pub status: &'static str,
    pub passing_rows: u64,
    pub failing_rows: u64,
    pub scope: &'static str,
}

#[derive(Debug, Serialize)]
pub struct ImpactArtifact {
    pub schema_version: &'static str,
    pub signal_schema_version: &'static str,
    pub generated_at: String,
    pub execution_mode: String,
    pub plan: ImpactPlan,
    pub execution: ImpactExecution,
    pub repository_completion: RepositoryCompletionMarker,
    pub aggregate: ImpactAggregate,
    pub rows: Vec<SignalRow>,
    pub findings: Vec<Findings>,
}

impl ImpactArtifact {
    #[must_use]
    pub fn new(
        generated_at: String,
        execution_mode: impl Into<String>,
        plan: ImpactPlan,
        mut issues: Vec<ImpactExecutionIssue>,
        rows: Vec<SignalRow>,
        findings: Vec<Findings>,
    ) -> Self {
        let passing_rows = rows.iter().filter(|row| row.pass).count() as u64;
        let failing_rows = rows.len() as u64 - passing_rows;
        let planned_jobs = plan.selected_checks.len() as u64;
        let completed_jobs = rows.len() as u64;
        if completed_jobs != planned_jobs && issues.is_empty() {
            issues.push(ImpactExecutionIssue {
                check: None,
                message: format!(
                    "impact execution produced {completed_jobs} rows for {planned_jobs} planned jobs"
                ),
            });
        }
        let state = if issues.is_empty() && completed_jobs == planned_jobs {
            ImpactExecutionState::Complete
        } else {
            ImpactExecutionState::Incomplete
        };
        Self {
            schema_version: IMPACT_SCHEMA_VERSION,
            signal_schema_version: crate::AYNI_SIGNAL_SCHEMA_VERSION,
            generated_at,
            execution_mode: execution_mode.into(),
            plan,
            execution: ImpactExecution {
                state,
                planned_jobs,
                completed_jobs,
                skipped_jobs: planned_jobs.saturating_sub(completed_jobs),
                issues,
            },
            repository_completion: RepositoryCompletionMarker {
                evaluated: false,
                required_command: "ayni check",
            },
            aggregate: ImpactAggregate {
                status: if state == ImpactExecutionState::Complete && failing_rows == 0 {
                    "pass"
                } else {
                    "fail"
                },
                passing_rows,
                failing_rows,
                scope: "selected_impact_plan_only",
            },
            rows,
            findings,
        }
    }

    #[must_use]
    pub fn outcome(&self) -> RunOutcome {
        if self.execution.state == ImpactExecutionState::Incomplete
            || self
                .rows
                .iter()
                .any(|row| row.result.command_failure().is_some())
        {
            RunOutcome::ExecutionIncomplete
        } else if self.aggregate.failing_rows > 0 {
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
        ImpactConfidence, ImpactIdentity, ImpactIdentityKind, ImpactReason, ImpactReasonKind,
        Language, SignalKind,
    };

    fn plan() -> ImpactPlan {
        ImpactPlan {
            base: ImpactIdentity {
                kind: ImpactIdentityKind::Revision,
                revision: "base".into(),
                requested: Some("main".into()),
                fingerprint: None,
            },
            candidate: ImpactIdentity {
                kind: ImpactIdentityKind::WorkingTree,
                revision: "head".into(),
                requested: None,
                fingerprint: Some("sha256:candidate".into()),
            },
            changes: vec![],
            selected_checks: vec![SelectedCheck::root(
                Language::Rust,
                ".".into(),
                SignalKind::Test,
                ImpactReason {
                    kind: ImpactReasonKind::ChangedFile,
                    detail: "change".into(),
                },
                ImpactConfidence::Certain,
            )],
            uncertainties: vec![],
            repository_completion_required: true,
        }
    }

    #[test]
    fn missing_selected_evidence_is_incomplete_even_without_caller_issue() {
        let artifact = ImpactArtifact::new("now".into(), "host", plan(), vec![], vec![], vec![]);
        assert_eq!(artifact.execution.state, ImpactExecutionState::Incomplete);
        assert_eq!(artifact.execution.planned_jobs, 1);
        assert_eq!(artifact.execution.completed_jobs, 0);
        assert_eq!(artifact.outcome(), RunOutcome::ExecutionIncomplete);
    }
}
