use crate::{discovery, policy, ui, verification_command};
use ayni_adapters_common::paths::validate_configured_root_containment;
use ayni_core::{
    AYNI_SIGNAL_SCHEMA_VERSION, AdapterRegistry, ArtifactToolVersion, AyniPolicy, Budget,
    CompletionIssue, CompletionScope, CompletionStage, CompletionState, ConcurrencyPolicy,
    ExecutionMode, InvocationContext, Language, OutputContext, RunArtifact, RunArtifactMetadata,
    RunCompletion, RunContext, RunOutcome, Scope, SignalKind, SignalResult, SignalRow,
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;

mod artifacts;
mod check;
mod execution;
mod reconciliation;

use artifacts::build_artifact_metadata;
pub(crate) use artifacts::{
    SIGNALS_ARTIFACT, VERIFY_SIGNALS_ARTIFACT, build_artifact_metadata_for_command,
    emit_analyze_outputs, persist_artifact_at, serialize_artifact, workspace_root_from_config_path,
};
pub(crate) use check::analyze;
pub(crate) use execution::AnalyzeOptions;
use execution::{build_analyze_plan, run_collect_with_ui};
pub(crate) use reconciliation::reconcile;

const MANAGED_TARGET_ENVIRONMENTS: &str = "AYNI_MANAGED_TARGET_ENVIRONMENTS";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OutputArg {
    Stdout,
    Md,
    Json,
}

impl OutputArg {
    fn as_str(self) -> &'static str {
        match self {
            Self::Stdout => "stdout",
            Self::Md => "md",
            Self::Json => "json",
        }
    }
}

pub(crate) fn enabled_signal_kinds(policy: &AyniPolicy) -> Vec<SignalKind> {
    let mut kinds = Vec::new();
    if policy.checks.test {
        kinds.push(SignalKind::Test);
    }
    if policy.checks.coverage {
        kinds.push(SignalKind::Coverage);
    }
    if policy.checks.size {
        kinds.push(SignalKind::Size);
    }
    if policy.checks.complexity {
        kinds.push(SignalKind::Complexity);
    }
    if policy.checks.deps {
        kinds.push(SignalKind::Deps);
    }
    if policy.checks.mutation {
        kinds.push(SignalKind::Mutation);
    }
    kinds
}

pub(crate) fn signal_kind_slug(kind: SignalKind) -> &'static str {
    match kind {
        SignalKind::Test => "test",
        SignalKind::Coverage => "coverage",
        SignalKind::Size => "size",
        SignalKind::Complexity => "complexity",
        SignalKind::Deps => "deps",
        SignalKind::Mutation => "mutation",
    }
}

fn tool_id(language: Language, root: &str, kind: SignalKind) -> String {
    format!("{}:{}:{}", language.as_str(), root, signal_kind_slug(kind))
}

#[derive(Clone, Debug)]
pub(crate) struct AnalyzeTarget {
    pub(crate) language: Language,
    pub(crate) root: String,
    pub(crate) run_context: RunContext,
}

#[derive(Clone, Debug)]
pub(crate) struct AnalyzePlanning {
    pub(crate) targets: Vec<AnalyzeTarget>,
    pub(crate) expected_targets: u64,
    pub(crate) detected_targets: u64,
    pub(crate) issues: Vec<CompletionIssue>,
}

impl AnalyzePlanning {
    fn completion(
        &self,
        scope: CompletionScope,
        completed_targets: u64,
        mut additional_issues: Vec<CompletionIssue>,
    ) -> RunCompletion {
        let mut issues = self.issues.clone();
        issues.append(&mut additional_issues);
        let skipped_targets = self.expected_targets - completed_targets;
        RunCompletion {
            scope,
            state: if skipped_targets == 0 {
                CompletionState::Complete
            } else {
                CompletionState::Incomplete
            },
            expected_targets: self.expected_targets,
            detected_targets: self.detected_targets,
            completed_targets,
            skipped_targets,
            issues,
        }
    }

    fn runnable_failure_issues(
        &self,
        stage: CompletionStage,
        message: &str,
    ) -> Vec<CompletionIssue> {
        self.targets
            .iter()
            .map(|target| CompletionIssue {
                language: target.language,
                configured_root: target.root.clone(),
                stage,
                message: message.to_string(),
            })
            .collect()
    }
}

pub(crate) fn build_analyze_targets(
    repo_root: &Path,
    policy: &AyniPolicy,
    package: Option<String>,
    file: Option<String>,
    language_filter: Option<Language>,
    debug: bool,
    registry: &AdapterRegistry,
) -> Result<AnalyzePlanning, String> {
    let file = file.map(|value| canonicalize_relative_posix(&value));
    let enabled_languages = policy.enabled_languages()?;
    if let Some(language) = language_filter
        && !enabled_languages.contains(&language)
    {
        return Err(format!(
            "requested language {language} is not enabled in the configured policy"
        ));
    }
    let configured =
        discovery::plan_configured_targets(repo_root, policy, language_filter, registry)?;
    let expected_targets = configured.len() as u64;
    let detected_targets = configured.iter().filter(|target| target.detected).count() as u64;
    let issues = configured
        .iter()
        .filter_map(|target| target.issue.clone())
        .collect();
    let mut targets = Vec::new();
    for configured_target in configured {
        let language = configured_target.language;
        let root = configured_target.configured_root;
        if let Some(mut execution) = configured_target.execution {
            if let Some(environment) = managed_target_environment(language, &root)? {
                execution.environment.extend(environment);
                execution
                    .environment
                    .insert(String::from(MANAGED_TARGET_ENVIRONMENTS), String::new());
            }
            let workdir = repo_root.join(&root);
            let scope = Scope {
                workspace_root: repo_root.to_string_lossy().into_owned(),
                path: if root == "." {
                    None
                } else {
                    Some(root.clone())
                },
                package: package.clone(),
                file: file.clone(),
            };
            let run_context = RunContext {
                repo_root: repo_root.to_path_buf(),
                target_root: workdir.clone(),
                workdir: workdir.clone(),
                policy: policy.clone(),
                scope,
                execution,
                debug,
            };
            targets.push(AnalyzeTarget {
                language,
                root,
                run_context,
            });
        }
    }
    Ok(AnalyzePlanning {
        targets,
        expected_targets,
        detected_targets,
        issues,
    })
}

pub(crate) fn managed_execution_active() -> bool {
    std::env::var_os(MANAGED_TARGET_ENVIRONMENTS).is_some_and(|value| !value.is_empty())
}

fn managed_target_environment(
    language: Language,
    root: &str,
) -> Result<Option<BTreeMap<String, String>>, String> {
    let Some(serialized) =
        std::env::var_os(MANAGED_TARGET_ENVIRONMENTS).filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };
    let environments: BTreeMap<String, BTreeMap<String, String>> =
        serde_json::from_str(&serialized.to_string_lossy())
            .map_err(|error| format!("managed target environments are invalid: {error}"))?;
    let key = format!("{language}:{root}");
    environments
        .get(&key)
        .cloned()
        .map(Some)
        .ok_or_else(|| format!("managed environment has no locked activation for target {key}"))
}

fn canonicalize_relative_posix(value: &str) -> String {
    let mut normalized = value.trim().replace('\\', "/");
    while normalized.ends_with('/') {
        normalized.pop();
    }
    if normalized.is_empty() {
        String::from(".")
    } else {
        normalized
    }
}
