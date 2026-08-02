//! Read-only install readiness planning.
//!
//! This module deliberately accepts an already-valid policy.  Scaffolding and
//! installation remain in `install`; this probe only asks adapters to detect
//! roots, resolve execution, and inspect catalog status.

use crate::discovery::{PlannedConfiguredTarget, plan_configured_targets_for_languages};
use crate::install::{catalog_entry_enabled_for_policy, catalog_failure, catalog_timeout};
use ayni_core::{
    AdapterRegistry, AyniPolicy, CatalogEntry, CompletionIssue, CompletionStage,
    ExecutionResolution, Language, LanguageAdapter, SignalKind, ToolStatus,
};
use serde::Serialize;
use std::collections::BTreeSet;
use std::path::Path;

pub(crate) const INSTALL_READINESS_VERSION: &str = "0.1.0";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum InstallReadinessState {
    Ready,
    NotReady,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct InstallReadiness {
    pub readiness_version: String,
    pub state: InstallReadinessState,
    pub targets: Vec<InstallReadinessTarget>,
    pub issues: Vec<InstallReadinessIssue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct InstallReadinessTarget {
    pub language: Language,
    pub configured_root: String,
    pub detection: InstallDetection,
    pub execution: Option<ExecutionResolution>,
    pub requirements: Vec<InstallRequirement>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct InstallDetection {
    pub detected: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct InstallRequirement {
    pub name: String,
    pub signals: Vec<SignalKind>,
    pub status: InstallRequirementStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum InstallRequirementStatus {
    Missing,
    Outdated,
    Current,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct InstallReadinessIssue {
    pub language: Language,
    pub configured_root: String,
    pub stage: InstallReadinessIssueStage,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requirement: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum InstallReadinessIssueStage {
    Detection,
    Resolution,
    Requirement,
}

/// Produce a deterministic, read-only readiness projection for configured
/// targets selected by `selected_languages`. An empty selection includes every
/// enabled language. The caller owns policy loading and validation.
pub(crate) fn probe_install_readiness(
    repo_root: &Path,
    policy: &AyniPolicy,
    selected_languages: &BTreeSet<Language>,
    registry: &AdapterRegistry,
) -> Result<InstallReadiness, String> {
    let planned =
        plan_configured_targets_for_languages(repo_root, policy, selected_languages, registry)?;
    let mut targets = Vec::with_capacity(planned.len());
    let mut issues = Vec::new();

    for planned_target in planned {
        targets.push(project_target(
            planned_target,
            policy,
            registry,
            &mut issues,
        ));
    }
    let state = if issues.is_empty() {
        InstallReadinessState::Ready
    } else {
        InstallReadinessState::NotReady
    };
    Ok(InstallReadiness {
        readiness_version: String::from(INSTALL_READINESS_VERSION),
        state,
        targets,
        issues,
    })
}

fn project_target(
    planned: PlannedConfiguredTarget,
    policy: &AyniPolicy,
    registry: &AdapterRegistry,
    issues: &mut Vec<InstallReadinessIssue>,
) -> InstallReadinessTarget {
    let detection = InstallDetection {
        detected: planned.detected,
        reason: planned.issue.as_ref().and_then(|issue| {
            (issue.stage == CompletionStage::Detection).then(|| issue.message.clone())
        }),
    };
    if let Some(issue) = planned.issue.as_ref() {
        issues.push(planning_issue(issue));
    }
    let requirements = planned
        .execution
        .as_ref()
        .map_or_else(Vec::new, |execution| {
            probe_requirements(
                planned.language,
                &planned.configured_root,
                execution,
                policy,
                registry,
                issues,
            )
        });
    InstallReadinessTarget {
        language: planned.language,
        configured_root: planned.configured_root,
        detection,
        execution: planned.execution,
        requirements,
    }
}

fn planning_issue(issue: &CompletionIssue) -> InstallReadinessIssue {
    InstallReadinessIssue {
        language: issue.language,
        configured_root: issue.configured_root.clone(),
        stage: match issue.stage {
            CompletionStage::Detection => InstallReadinessIssueStage::Detection,
            CompletionStage::Resolution => InstallReadinessIssueStage::Resolution,
            _ => {
                unreachable!("configured-target planner only emits detection or resolution issues")
            }
        },
        message: issue.message.clone(),
        requirement: None,
    }
}

fn probe_requirements(
    language: Language,
    configured_root: &str,
    execution: &ExecutionResolution,
    policy: &AyniPolicy,
    registry: &AdapterRegistry,
    issues: &mut Vec<InstallReadinessIssue>,
) -> Vec<InstallRequirement> {
    let adapter = registry
        .adapters()
        .iter()
        .find(|adapter| adapter.language() == language)
        .expect("configured-target planner resolved execution only for registered adapters");
    let mut requirements = Vec::new();
    for entry in adapter
        .catalog()
        .iter()
        .filter(|entry| catalog_entry_enabled_for_policy(policy, entry))
    {
        let (requirement, issue) = probe_requirement(
            adapter.as_ref(),
            entry,
            execution,
            language,
            configured_root,
            policy,
        );
        if let Some(issue) = issue {
            issues.push(issue);
        }
        requirements.push(requirement);
    }
    requirements
}

fn probe_requirement(
    adapter: &dyn LanguageAdapter,
    entry: &CatalogEntry,
    execution: &ExecutionResolution,
    language: Language,
    configured_root: &str,
    policy: &AyniPolicy,
) -> (InstallRequirement, Option<InstallReadinessIssue>) {
    let (status, diagnostic) =
        match adapter
            .catalog_runtime()
            .status(entry, execution, catalog_timeout(policy))
        {
            Ok(status) => (readiness_status(status), None),
            Err(error) => (
                InstallRequirementStatus::Missing,
                Some(catalog_failure(
                    language,
                    configured_root,
                    entry.name,
                    &error,
                )),
            ),
        };
    let issue = (status != InstallRequirementStatus::Current || diagnostic.is_some()).then(|| {
        InstallReadinessIssue {
            language,
            configured_root: configured_root.to_string(),
            stage: InstallReadinessIssueStage::Requirement,
            message: diagnostic
                .unwrap_or_else(|| format!("{} is {}", entry.name, readiness_status_name(status))),
            requirement: Some(entry.name.to_string()),
        }
    });
    (
        InstallRequirement {
            name: entry.name.to_string(),
            signals: entry.for_signals.to_vec(),
            status,
        },
        issue,
    )
}

/// Load the existing policy and emit one readiness report. This path never
/// calls install preparation, scaffolding, validation, or artifact writers.
pub(crate) fn run(
    repo_root: &Path,
    selected_languages: &BTreeSet<Language>,
    json: bool,
    registry: &AdapterRegistry,
) -> Result<bool, String> {
    let policy = AyniPolicy::load(repo_root)?;
    let readiness = probe_install_readiness(repo_root, &policy, selected_languages, registry)?;
    if json {
        let serialized = serde_json::to_string_pretty(&readiness)
            .map_err(|error| format!("failed to serialize install readiness: {error}"))?;
        println!("{serialized}");
    } else {
        print_human(&readiness);
    }
    Ok(readiness.state == InstallReadinessState::Ready)
}

fn print_human(readiness: &InstallReadiness) {
    println!(
        "Ayni install readiness {} — {}",
        readiness.readiness_version,
        state_name(readiness.state)
    );
    for target in &readiness.targets {
        println!("\n{}:{}", target.language, target.configured_root);
        println!("  detected: {}", target.detection.detected);
        if let Some(reason) = &target.detection.reason {
            println!("  detection: {reason}");
        }
        if let Some(execution) = &target.execution {
            println!(
                "  resolution: runner={} source={} kind={} resolved_from={} confidence={} ambiguous={} install_cwd={} exec_cwd={}",
                execution.runner,
                execution.source,
                execution.kind,
                execution.resolved_from.display(),
                execution.confidence,
                execution.ambiguous,
                execution.install_cwd.display(),
                execution.exec_cwd.display()
            );
        } else {
            println!("  resolution: unresolved");
        }
        for requirement in &target.requirements {
            println!(
                "  requirement: {} status={} signals={}",
                requirement.name,
                requirement_status_name(requirement.status),
                requirement
                    .signals
                    .iter()
                    .map(|signal| crate::signal_kind_slug(*signal))
                    .collect::<Vec<_>>()
                    .join(",")
            );
        }
    }
    if !readiness.issues.is_empty() {
        println!("\nIssues:");
        for issue in &readiness.issues {
            println!(
                "  {}:{} [{}] {}",
                issue.language,
                issue.configured_root,
                issue_stage_name(issue.stage),
                issue.message
            );
        }
    }
}

fn state_name(state: InstallReadinessState) -> &'static str {
    match state {
        InstallReadinessState::Ready => "ready",
        InstallReadinessState::NotReady => "not_ready",
    }
}

fn requirement_status_name(status: InstallRequirementStatus) -> &'static str {
    readiness_status_name(status)
}

fn issue_stage_name(stage: InstallReadinessIssueStage) -> &'static str {
    match stage {
        InstallReadinessIssueStage::Detection => "detection",
        InstallReadinessIssueStage::Resolution => "resolution",
        InstallReadinessIssueStage::Requirement => "requirement",
    }
}

fn readiness_status(status: ToolStatus) -> InstallRequirementStatus {
    match status {
        ToolStatus::Missing => InstallRequirementStatus::Missing,
        ToolStatus::Outdated => InstallRequirementStatus::Outdated,
        ToolStatus::Current => InstallRequirementStatus::Current,
    }
}

fn readiness_status_name(status: InstallRequirementStatus) -> &'static str {
    match status {
        InstallRequirementStatus::Missing => "missing",
        InstallRequirementStatus::Outdated => "outdated",
        InstallRequirementStatus::Current => "current",
    }
}

#[cfg(test)]
mod tests {
    use super::{
        InstallReadinessState, InstallRequirementStatus, probe_install_readiness, readiness_status,
    };
    use ayni_core::{AyniPolicy, Language, ToolStatus};
    use std::collections::BTreeSet;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn undetected_configured_target_is_not_ready_without_writing() {
        let directory = TempDir::new().expect("tempdir");
        fs::write(directory.path().join("sentinel"), "unchanged").expect("sentinel");
        let policy: AyniPolicy = toml::from_str(
            "[checks]\ntest = true\n[languages]\nenabled = [\"rust\"]\n[rust]\nroots = [\"missing\"]\n",
        ).expect("policy");

        let readiness = probe_install_readiness(
            directory.path(),
            &policy,
            &BTreeSet::from([Language::Rust]),
            &crate::build_registry(),
        )
        .expect("readiness");

        assert_eq!(readiness.state, InstallReadinessState::NotReady);
        assert_eq!(readiness.targets.len(), 1);
        assert!(!readiness.targets[0].detection.detected);
        assert_eq!(readiness.issues.len(), 1);
        assert_eq!(
            fs::read_to_string(directory.path().join("sentinel")).expect("sentinel"),
            "unchanged"
        );
        assert!(!directory.path().join(".ayni.toml").exists());
        assert!(!directory.path().join(".gitignore").exists());
    }

    #[test]
    fn missing_and_outdated_requirements_are_not_current() {
        assert_eq!(
            readiness_status(ToolStatus::Missing),
            InstallRequirementStatus::Missing
        );
        assert_eq!(
            readiness_status(ToolStatus::Outdated),
            InstallRequirementStatus::Outdated
        );
        assert_eq!(
            readiness_status(ToolStatus::Current),
            InstallRequirementStatus::Current
        );
    }
}
