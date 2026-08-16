use super::*;

pub(crate) const SIGNALS_ARTIFACT: &str = ".ayni/last/signals.json";
pub(crate) const VERIFY_SIGNALS_ARTIFACT: &str = ".ayni/verify/last/signals.json";
pub(super) const ARTIFACTS_DIR: &str = ".ayni/last";

pub(super) fn build_artifact_metadata(
    config_path: &Path,
    workspace_root: &Path,
    planning: &AnalyzePlanning,
    output_mode: OutputArg,
) -> Result<RunArtifactMetadata, String> {
    build_artifact_metadata_for_command(config_path, workspace_root, planning, output_mode, "check")
}

pub(crate) fn build_artifact_metadata_for_command(
    config_path: &Path,
    workspace_root: &Path,
    planning: &AnalyzePlanning,
    output_mode: OutputArg,
    command: &str,
) -> Result<RunArtifactMetadata, String> {
    let generated_at = time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .map_err(|error| format!("failed to format analysis timestamp: {error}"))?;
    let languages = planning
        .targets
        .iter()
        .map(|target| target.language)
        .chain(planning.issues.iter().map(|issue| issue.language))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let scope = planning
        .targets
        .first()
        .map(|target| target.run_context.scope.clone());
    Ok(RunArtifactMetadata {
        generated_at,
        ayni_version: String::from(env!("CARGO_PKG_VERSION")),
        invocation: InvocationContext {
            command: command.to_string(),
            languages,
            scope,
        },
        output: OutputContext {
            format: output_mode.as_str().to_string(),
            destination: String::from("stdout"),
        },
        config_path: config_path.to_string_lossy().into_owned(),
        repository_root: workspace_root.to_string_lossy().into_owned(),
    })
}

pub(crate) fn serialize_artifact(artifact: &RunArtifact) -> Result<String, String> {
    serde_json::to_string_pretty(artifact)
        .map(|serialized| format!("{serialized}\n"))
        .map_err(|error| format!("failed to serialize artifact: {error}"))
}

pub(crate) fn persist_artifact_at(
    repo_root: &Path,
    relative_path: &str,
    serialized: &str,
) -> Result<(), String> {
    let destination = repo_root.join(relative_path);
    let parent = destination
        .parent()
        .ok_or_else(|| format!("artifact path {relative_path} has no parent"))?;
    fs::create_dir_all(parent).map_err(|error| {
        format!("failed to create artifact directory for {relative_path}: {error}")
    })?;
    let file_name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("artifact path {relative_path} has no file name"))?;
    let temporary = parent.join(format!(".{file_name}.tmp-{}", std::process::id()));
    fs::write(&temporary, serialized).map_err(|error| {
        format!("failed to write temporary artifact for {relative_path}: {error}")
    })?;
    if let Err(error) = fs::rename(&temporary, &destination) {
        let _ = fs::remove_file(&temporary);
        return Err(format!(
            "failed to atomically replace {relative_path}: {error}"
        ));
    }
    Ok(())
}

pub(crate) fn emit_analyze_outputs(
    output_mode: OutputArg,
    policy: &AyniPolicy,
    artifact: &RunArtifact,
    serialized: &str,
) -> Result<(), String> {
    match output_mode {
        OutputArg::Stdout => {
            ui::report::print_from_artifact(artifact, policy.report.offenders_limit);
        }
        OutputArg::Md => {
            ui::progress_log::log_command_failures(artifact);
            let summary = ui::md_report::build_markdown(artifact, policy.report.offenders_limit);
            println!("{summary}");
        }
        OutputArg::Json => {
            print!("{serialized}");
        }
    }
    Ok(())
}

pub(crate) fn workspace_root_from_config_path(config_path: &Path) -> PathBuf {
    let Some(parent) = config_path.parent() else {
        return PathBuf::from(".");
    };
    if parent.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        parent.to_path_buf()
    }
}
