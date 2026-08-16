use ayni_adapters_common::paths::validate_configured_root_containment;
use ayni_core::{
    AdapterRegistry, AyniPolicy, CompletionIssue, CompletionStage, ExecutionResolution, Language,
};
use std::collections::BTreeSet;
use std::path::Path;

#[derive(Debug)]
pub struct PlannedConfiguredTarget {
    pub language: Language,
    pub configured_root: String,
    pub detected: bool,
    pub execution: Option<ExecutionResolution>,
    pub issue: Option<CompletionIssue>,
}

/// Plans every configured target selected by the command without dropping an
/// undetected or unresolvable root. Detection and execution resolution remain
/// adapter-owned; this function only records their outcomes for CLI orchestration.
pub fn plan_configured_targets(
    repo_root: &Path,
    policy: &AyniPolicy,
    language_filter: Option<Language>,
    registry: &AdapterRegistry,
) -> Result<Vec<PlannedConfiguredTarget>, String> {
    let selected_languages = language_filter.into_iter().collect();
    plan_configured_targets_for_languages(repo_root, policy, &selected_languages, registry)
}

/// Plans configured targets for the selected languages. An empty selection
/// includes all enabled languages, matching install's selection semantics.
pub fn plan_configured_targets_for_languages(
    repo_root: &Path,
    policy: &AyniPolicy,
    selected_languages: &BTreeSet<Language>,
    registry: &AdapterRegistry,
) -> Result<Vec<PlannedConfiguredTarget>, String> {
    validate_configured_root_containment(repo_root, policy)?;
    let mut planned = Vec::new();
    for language in policy.enabled_languages()? {
        if !selected_languages.is_empty() && !selected_languages.contains(&language) {
            continue;
        }
        for configured_root in policy.roots_for(language) {
            let root_path = repo_root.join(configured_root);
            let Some(adapter) = registry
                .adapters()
                .iter()
                .find(|adapter| adapter.language() == language)
            else {
                planned.push(PlannedConfiguredTarget {
                    language,
                    configured_root: configured_root.clone(),
                    detected: false,
                    execution: None,
                    issue: Some(CompletionIssue {
                        language,
                        configured_root: configured_root.clone(),
                        stage: CompletionStage::Detection,
                        message: format!("{language} adapter is unavailable"),
                    }),
                });
                continue;
            };

            let detection = adapter.detect(&root_path);
            if !detection.detected {
                planned.push(PlannedConfiguredTarget {
                    language,
                    configured_root: configured_root.clone(),
                    detected: false,
                    execution: None,
                    issue: Some(CompletionIssue {
                        language,
                        configured_root: configured_root.clone(),
                        stage: CompletionStage::Detection,
                        message: detection.reason.unwrap_or_else(|| {
                            format!(
                                "configured {language} root was not detected at {}",
                                root_path.display()
                            )
                        }),
                    }),
                });
                continue;
            }

            let execution = adapter.resolve_execution(repo_root, &root_path);
            let issue = execution.is_none().then(|| CompletionIssue {
                language,
                configured_root: configured_root.clone(),
                stage: CompletionStage::Resolution,
                message: format!(
                    "unable to resolve execution for configured {language} root {}",
                    root_path.display()
                ),
            });
            planned.push(PlannedConfiguredTarget {
                language,
                configured_root: configured_root.clone(),
                detected: true,
                execution,
                issue,
            });
        }
    }
    Ok(planned)
}
