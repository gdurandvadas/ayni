use crate::analysis::{
    AnalyzePlanning, OutputArg, VERIFY_SIGNALS_ARTIFACT, build_analyze_targets,
    build_artifact_metadata_for_command, emit_analyze_outputs, invalidate_artifact_at,
    managed_execution_active, persist_artifact_at, serialize_artifact, signal_kind_slug,
    workspace_root_from_config_path,
};
use crate::policy::load_from_path;
use crate::ui::cancellation::SignalCancellation;
use crate::{build_registry, verification_command};
use ayni_adapters_common::paths::validate_configured_root_containment;
use ayni_core::{
    AdapterRegistry, AyniPolicy, CompletionScope, CompletionStage, Language, RunArtifact,
    RunArtifactMetadata, RunOutcome, SignalKind, SignalRow, VerificationSelection,
};
use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

pub(crate) struct Request {
    pub kind: SignalKind,
    pub config_path: PathBuf,
    pub file: Option<String>,
    pub package: Option<String>,
    pub name: Option<String>,
    pub language: Option<Language>,
    pub root: Option<String>,
    pub output_mode: OutputArg,
    pub debug: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct ConfiguredTarget {
    language: Language,
    root: String,
}

pub(crate) type Error = crate::application_error::ApplicationError;

pub(crate) fn run(mut request: Request) -> Result<RunOutcome, Error> {
    let signal_cancellation = SignalCancellation::install().map_err(Error::execution)?;
    let (workspace_root, policy) = prepare_request(&mut request).map_err(Error::input)?;
    let (registry, mut planning) =
        plan_verification(&workspace_root, &policy, &request).map_err(Error::input)?;
    for target in &mut planning.targets {
        target.run_context.cancellation = signal_cancellation.token();
    }
    let artifact =
        match build_verification_artifact(&workspace_root, &registry, &planning, &request) {
            Ok(artifact) => artifact,
            Err(error) => {
                invalidate_artifact_at(&workspace_root, VERIFY_SIGNALS_ARTIFACT)
                    .map_err(Error::execution)?;
                return Err(Error::execution(error));
            }
        };
    if signal_cancellation.interrupted() {
        return Err(Error::execution("verification aborted by Ctrl-C"));
    }
    persist_and_emit_verification(
        artifact,
        &workspace_root,
        &policy,
        &registry,
        &planning,
        &request,
    )
    .map_err(Error::execution)
}

fn prepare_request(request: &mut Request) -> Result<(PathBuf, AyniPolicy), String> {
    let workspace_root = workspace_root_from_config_path(&request.config_path)?;
    // A focused artifact is current only after this invocation completes.
    invalidate_artifact_at(&workspace_root, VERIFY_SIGNALS_ARTIFACT)?;
    let policy = load_from_path(&request.config_path)?;
    validate_configured_root_containment(&workspace_root, &policy)?;
    validate_signal_enabled(&policy, request.kind)?;
    validate_selector_shape(request)?;

    if let Some(file) = request.file.take() {
        request.file = Some(validate_file_selector(&workspace_root, &file)?);
    }

    Ok((workspace_root, policy))
}

fn plan_verification(
    workspace_root: &Path,
    policy: &AyniPolicy,
    request: &Request,
) -> Result<(AdapterRegistry, AnalyzePlanning), String> {
    let registry = build_registry();
    let selected = select_configured_targets(workspace_root, policy, &registry, request)?;
    validate_adapter_support(&registry, &selected, request)?;

    let selected_language = selected
        .first()
        .expect("target selection is non-empty")
        .language;
    let mut planning = build_analyze_targets(
        workspace_root,
        policy,
        request.package.clone(),
        request.file.clone(),
        Some(selected_language),
        request.debug,
        &registry,
    )?;
    retain_selected_targets(&mut planning, &selected);
    Ok((registry, planning))
}

fn build_verification_artifact(
    workspace_root: &Path,
    registry: &AdapterRegistry,
    planning: &AnalyzePlanning,
    request: &Request,
) -> Result<RunArtifact, String> {
    let rows = collect_rows(planning, registry, request);
    let (completion, rows) = crate::analysis::reconcile(
        planning,
        CompletionScope::Requested,
        Some(request.kind),
        rows,
    );
    RunArtifact::new(
        build_artifact_metadata_for_command(
            &request.config_path,
            workspace_root,
            planning,
            request.output_mode,
            &format!("verify_{}", signal_kind_slug(request.kind)),
        )?,
        completion,
        rows,
    )
}

fn persist_and_emit_verification(
    mut artifact: RunArtifact,
    workspace_root: &Path,
    policy: &AyniPolicy,
    registry: &AdapterRegistry,
    planning: &AnalyzePlanning,
    request: &Request,
) -> Result<RunOutcome, String> {
    if let Err(error) = verification_command::materialize_finding_commands(
        &mut artifact,
        registry,
        !managed_execution_active(),
    ) {
        persist_incomplete_verification_artifact(
            workspace_root,
            planning,
            &artifact.metadata,
            &error,
        )?;
        return Err(error);
    }
    let serialized = match serialize_artifact(&artifact) {
        Ok(serialized) => serialized,
        Err(error) => {
            persist_incomplete_verification_artifact(
                workspace_root,
                planning,
                &artifact.metadata,
                &error,
            )?;
            return Err(error);
        }
    };
    persist_artifact_at(workspace_root, VERIFY_SIGNALS_ARTIFACT, &serialized)?;
    emit_analyze_outputs(request.output_mode, policy, &artifact, &serialized)?;

    Ok(artifact.outcome())
}

fn persist_incomplete_verification_artifact(
    workspace_root: &Path,
    planning: &AnalyzePlanning,
    metadata: &RunArtifactMetadata,
    message: &str,
) -> Result<(), String> {
    let artifact = RunArtifact::new(
        metadata.clone(),
        planning.completion(
            CompletionScope::Requested,
            0,
            planning.runnable_failure_issues(CompletionStage::Collection, message),
        ),
        Vec::new(),
    )?;
    let serialized = serialize_artifact(&artifact)?;
    persist_artifact_at(workspace_root, VERIFY_SIGNALS_ARTIFACT, &serialized)
}

fn validate_signal_enabled(policy: &AyniPolicy, kind: SignalKind) -> Result<(), String> {
    let enabled = match kind {
        SignalKind::Test => policy.checks.test,
        SignalKind::Coverage => policy.checks.coverage,
        SignalKind::Size => policy.checks.size,
        SignalKind::Complexity => policy.checks.complexity,
        SignalKind::Deps => policy.checks.deps,
        SignalKind::Mutation => policy.checks.mutation,
    };
    if enabled {
        Ok(())
    } else {
        Err(format!(
            "{} verification is disabled by the configured policy",
            signal_kind_slug(kind)
        ))
    }
}

fn validate_selector_shape(request: &Request) -> Result<(), String> {
    if request.file.is_some() && request.package.is_some() {
        return Err(String::from(
            "verification cannot combine --file and --package",
        ));
    }
    if request.kind != SignalKind::Test && request.name.is_some() {
        return Err(String::from("--name is valid only for verify test"));
    }
    Ok(())
}

fn validate_file_selector(repo_root: &Path, value: &str) -> Result<String, String> {
    let normalized = normalize_file_selector(value)?;
    canonical_repository_file(repo_root, &normalized)
}

fn normalize_file_selector(value: &str) -> Result<PathBuf, String> {
    let portable = value.trim().replace('\\', "/");
    let path = Path::new(&portable);
    let has_windows_prefix = portable.as_bytes().get(1) == Some(&b':')
        && portable
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphabetic);
    if portable.is_empty() || path.is_absolute() || has_windows_prefix {
        return Err(format!(
            "--file must be a non-empty repository-relative path: {value:?}"
        ));
    }
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => normalized.push(value),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(format!(
                    "--file must stay within the repository and cannot contain traversal: {value}"
                ));
            }
        }
    }
    if normalized.as_os_str().is_empty() {
        return Err(String::from("--file must identify a repository file"));
    }

    Ok(normalized)
}

fn canonical_repository_file(repo_root: &Path, normalized: &Path) -> Result<String, String> {
    let canonical_root = repo_root
        .canonicalize()
        .map_err(|error| format!("failed to resolve repository root: {error}"))?;
    let canonical_file = repo_root.join(normalized).canonicalize().map_err(|error| {
        format!(
            "--file {} does not resolve to a configured repository file: {error}",
            normalized.display()
        )
    })?;
    if !canonical_file.is_file() || !canonical_file.starts_with(&canonical_root) {
        return Err(format!(
            "--file {} must resolve to a file inside the repository",
            normalized.display()
        ));
    }
    let relative = canonical_file
        .strip_prefix(canonical_root)
        .map_err(|_| String::from("--file resolved outside the repository"))?;
    Ok(relative.to_string_lossy().replace('\\', "/"))
}

fn select_configured_targets(
    repo_root: &Path,
    policy: &AyniPolicy,
    registry: &AdapterRegistry,
    request: &Request,
) -> Result<Vec<ConfiguredTarget>, String> {
    let enabled = policy.enabled_languages()?;
    validate_requested_language(&enabled, request.language)?;
    validate_requested_root(policy, request, &enabled)?;

    let selected = if let Some(file) = request.file.as_deref() {
        select_file_targets(
            repo_root,
            policy,
            registry,
            request.language,
            request.root.as_deref(),
            &enabled,
            file,
        )?
    } else {
        select_non_file_targets(policy, request, &enabled)?
    };

    if selected.is_empty() {
        Err(String::from(
            "no configured verification target matched the request",
        ))
    } else {
        Ok(selected)
    }
}

fn validate_requested_language(
    enabled: &[Language],
    requested: Option<Language>,
) -> Result<(), String> {
    if let Some(language) = requested
        && !enabled.contains(&language)
    {
        return Err(format!(
            "requested language {language} is not enabled in the configured policy"
        ));
    }
    Ok(())
}

fn validate_requested_root(
    policy: &AyniPolicy,
    request: &Request,
    enabled: &[Language],
) -> Result<(), String> {
    let Some(root) = request.root.as_deref() else {
        return Ok(());
    };
    let matches = enabled.iter().copied().any(|language| {
        request.language.is_none_or(|selected| selected == language)
            && policy.roots_for(language).iter().any(|value| value == root)
    });
    if matches {
        Ok(())
    } else {
        Err(format!(
            "--root {root:?} is not a normalized configured root for the selected language"
        ))
    }
}

fn select_file_targets(
    repo_root: &Path,
    policy: &AyniPolicy,
    registry: &AdapterRegistry,
    requested_language: Option<Language>,
    requested_root: Option<&str>,
    enabled: &[Language],
    file: &str,
) -> Result<Vec<ConfiguredTarget>, String> {
    let mut selected = BTreeSet::new();
    for &language in enabled {
        if requested_language.is_some_and(|filter| filter != language) {
            continue;
        }
        let adapter = registry
            .adapters()
            .iter()
            .find(|adapter| adapter.language() == language)
            .ok_or_else(|| format!("{language} adapter unavailable"))?;
        if !profile_matches_file(&adapter.profile().default_file_globs, file) {
            continue;
        }
        for root in policy.roots_for(language) {
            if request_root_mismatch(root, requested_root) {
                continue;
            }
            if file_belongs_to_root(repo_root, file, root)? {
                selected.insert(ConfiguredTarget {
                    language,
                    root: root.clone(),
                });
            }
        }
    }
    if selected.len() > 1 {
        return Err(format!(
            "--file {file} is ambiguous across configured language/root targets; pass --language and --root"
        ));
    }
    Ok(selected.into_iter().collect())
}

fn select_non_file_targets(
    policy: &AyniPolicy,
    request: &Request,
    enabled: &[Language],
) -> Result<Vec<ConfiguredTarget>, String> {
    let language = resolve_non_file_language(enabled, request.language)?;
    let selected = policy
        .roots_for(language)
        .iter()
        .filter(|root| !request_root_mismatch(root, request.root.as_deref()))
        .cloned()
        .map(|root| ConfiguredTarget { language, root })
        .collect::<BTreeSet<_>>();
    let has_narrow_selector = request.package.is_some() || request.name.is_some();
    if has_narrow_selector && selected.len() > 1 {
        return Err(String::from(
            "package or name verification is ambiguous across configured roots; pass --root",
        ));
    }
    Ok(selected.into_iter().collect())
}

fn request_root_mismatch(configured: &str, requested: Option<&str>) -> bool {
    requested.is_some_and(|requested| requested != configured)
}

fn resolve_non_file_language(
    enabled: &[Language],
    requested: Option<Language>,
) -> Result<Language, String> {
    if let Some(language) = requested {
        return Ok(language);
    }
    let unique = enabled.iter().copied().collect::<BTreeSet<_>>();
    if unique.len() == 1 {
        return Ok(*unique.first().expect("one enabled language"));
    }
    Err(String::from(
        "--language is required when verification matches multiple configured languages",
    ))
}

fn profile_matches_file(globs: &[String], file: &str) -> bool {
    let file_name = Path::new(file)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(file);
    globs.iter().any(|pattern| {
        pattern
            .strip_prefix('*')
            .is_some_and(|suffix| file_name.ends_with(suffix))
            || pattern == file_name
    })
}

fn file_belongs_to_root(
    repo_root: &Path,
    file: &str,
    configured_root: &str,
) -> Result<bool, String> {
    if configured_root != "."
        && file != configured_root
        && !file.starts_with(&format!("{configured_root}/"))
    {
        return Ok(false);
    }
    let root = repo_root.join(configured_root).canonicalize().map_err(|error| {
        format!(
            "configured root {configured_root} cannot be resolved while selecting --file: {error}"
        )
    })?;
    let file = repo_root
        .join(file)
        .canonicalize()
        .map_err(|error| format!("selected file cannot be resolved: {error}"))?;
    Ok(file.starts_with(root))
}

fn validate_adapter_support(
    registry: &AdapterRegistry,
    selected: &[ConfiguredTarget],
    request: &Request,
) -> Result<(), String> {
    let selection = VerificationSelection {
        file: request.file.clone(),
        package: request.package.clone(),
        name: request.name.clone(),
    };
    let languages = selected
        .iter()
        .map(|target| target.language)
        .collect::<BTreeSet<_>>();
    for language in languages {
        let adapter = registry
            .adapters()
            .iter()
            .find(|adapter| adapter.language() == language)
            .ok_or_else(|| format!("{language} adapter unavailable"))?;
        adapter
            .validate_verification_selection(request.kind, &selection)
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn retain_selected_targets(planning: &mut AnalyzePlanning, selected: &[ConfiguredTarget]) {
    let selected = selected.iter().cloned().collect::<BTreeSet<_>>();
    planning.targets.retain(|target| {
        selected.contains(&ConfiguredTarget {
            language: target.language,
            root: target.root.clone(),
        })
    });
    planning.issues.retain(|issue| {
        selected.contains(&ConfiguredTarget {
            language: issue.language,
            root: issue.configured_root.clone(),
        })
    });
    planning.expected_targets = selected.len() as u64;
    planning.detected_targets = planning.targets.len() as u64
        + planning
            .issues
            .iter()
            .filter(|issue| issue.stage != ayni_core::CompletionStage::Detection)
            .count() as u64;
}

fn collect_rows(
    planning: &AnalyzePlanning,
    registry: &AdapterRegistry,
    request: &Request,
) -> Vec<SignalRow> {
    let mut rows = Vec::new();
    for target in &planning.targets {
        if target.run_context.cancellation.is_cancelled() {
            break;
        }
        let selection = VerificationSelection {
            file: target.run_context.scope.file.clone(),
            package: target.run_context.scope.package.clone(),
            name: request.name.clone(),
        };
        let adapter = registry
            .adapters()
            .iter()
            .find(|adapter| adapter.language() == target.language)
            .expect("selected adapter remains registered");
        match adapter.collect_verification(
            request.kind,
            &target.run_context,
            &selection,
            &mut |line| {
                if request.debug {
                    eprintln!("[{}] {line}", target.language);
                }
            },
        ) {
            Ok(row) => rows.push(row),
            Err(error) => {
                if request.debug {
                    eprintln!("[{}] collection incomplete: {error}", target.language);
                }
            }
        }
        if target.run_context.cancellation.is_cancelled() {
            break;
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::{resolve_non_file_language, validate_file_selector};
    use ayni_core::Language;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn file_selector_rejects_absolute_and_traversal_paths() {
        let root = TempDir::new().expect("tempdir");
        assert!(validate_file_selector(root.path(), "/tmp/test.rs").is_err());
        assert!(validate_file_selector(root.path(), "../test.rs").is_err());
    }

    #[test]
    fn file_selector_normalizes_an_existing_repository_file() {
        let root = TempDir::new().expect("tempdir");
        fs::create_dir(root.path().join("src")).expect("src");
        fs::write(root.path().join("src/lib.rs"), "").expect("source");
        assert_eq!(
            validate_file_selector(root.path(), "./src/lib.rs").expect("valid selector"),
            "src/lib.rs"
        );
    }

    #[test]
    fn duplicate_configured_languages_resolve_as_one_target_language() {
        assert_eq!(
            resolve_non_file_language(&[Language::Rust, Language::Rust], None)
                .expect("one unique language"),
            Language::Rust
        );
    }
}
