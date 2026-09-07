use crate::environment::{find_jacoco, find_plugin, gradle_owner};
use ayni_adapters_common::{
    repository::{read_contained_string, repository_relative},
    tooling::{baseline_plan, diagnostic, finish},
};
use ayni_core::{AdapterError, Language, SignalKind, ToolingPlan, ToolingRequest};

pub(crate) fn plan(request: &ToolingRequest) -> Result<ToolingPlan, AdapterError> {
    let mut plan = baseline_plan(
        request,
        crate::catalog::KOTLIN_CATALOG,
        crate::tooling::KOTLIN_TOOLS,
    )?;
    if plan.tools.is_empty() {
        return Ok(plan);
    }
    inspect(request, &mut plan).unwrap_or_else(|cause| {
        plan.conflicts.push(diagnostic(
            "tooling.native_metadata",
            cause.to_string(),
            None,
        ))
    });
    finish(&mut plan);
    Ok(plan)
}
fn error(cause: impl std::fmt::Display) -> AdapterError {
    AdapterError::new(Language::Kotlin, cause.to_string())
}
fn inspect(request: &ToolingRequest, plan: &mut ToolingPlan) -> Result<(), AdapterError> {
    let repo = request.repo_root();
    let owner = gradle_owner(repo, &repo.join(&request.target().root))?;
    plan.owner_root = repository_relative(repo, &owner).map_err(error)?;
    let (scripts, has_lock) = inspect_scripts(repo, &owner, &mut plan.conflicts)?;
    let metadata = ayni_adapters_common::repository::read_optional_contained_string(
        repo,
        &owner.join("gradle/verification-metadata.xml"),
    )
    .map_err(error)?;
    let has_integrity = metadata.as_deref().is_some_and(|text| {
        text.contains("<verification-metadata") && text.contains("<sha256 value=")
    });
    let (kover, jacoco) = select_coverage(request, plan, &scripts)?;
    for tool in &mut plan.tools {
        let found = declaration(&tool.tool, &scripts, &kover, &jacoco)?;
        if let Some((path, version)) = found {
            tool.declaration_path = Some(path);
            tool.current_declaration = Some(version.clone());
            // A literal plugin declaration alone is not downloaded-artifact evidence.
            if has_lock && has_integrity {
                tool.current_resolution = Some(version);
            }
        } else {
            tool.declaration_path = Some(scripts[0].0.clone());
        }
    }
    if !plan.tools.is_empty() && (!has_lock || !has_integrity) {
        plan.conflicts.push(diagnostic("tooling.gradle_integrity_required", "Gradle dependency locks and verification metadata are required; refresh them after changing plugins", None));
    }
    Ok(())
}

fn inspect_scripts(
    repo: &std::path::Path,
    owner: &std::path::Path,
    conflicts: &mut Vec<ayni_core::ToolingDiagnostic>,
) -> Result<(Vec<(String, String)>, bool), AdapterError> {
    let mut scripts = Vec::new();
    let mut has_lock = false;
    for entry in walkdir::WalkDir::new(owner)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            !matches!(
                e.file_name().to_str(),
                Some(".git" | ".ayni" | ".gradle" | "build" | "out")
            )
        })
    {
        let entry = entry.map_err(error)?;
        let name = entry.file_name().to_string_lossy();
        if name == "gradle.lockfile" || name.ends_with(".lockfile") {
            let content = read_contained_string(repo, entry.path()).map_err(error)?;
            if content.trim().is_empty() {
                return Err(error("Gradle dependency lock must not be empty"));
            }
            has_lock = true;
        }
        if !matches!(
            name.as_ref(),
            "build.gradle" | "build.gradle.kts" | "settings.gradle" | "settings.gradle.kts"
        ) {
            continue;
        }
        let content = read_contained_string(repo, entry.path()).map_err(error)?;
        let path = repository_relative(repo, entry.path()).map_err(error)?;
        if content.contains("alias(") || content.contains("alias (") {
            conflicts.push(diagnostic(
                "tooling.unsupported_gradle_shape",
                "version-catalog plugin aliases require manual reconciliation",
                Some(path.clone()),
            ));
        }
        scripts.push((path, content));
    }
    if scripts.is_empty() {
        return Err(error("no direct Gradle build declarations found"));
    }
    scripts.sort();
    Ok((scripts, has_lock))
}

type PluginDeclaration = Option<(String, String)>;
fn select_coverage(
    request: &ToolingRequest,
    plan: &mut ToolingPlan,
    scripts: &[(String, String)],
) -> Result<(PluginDeclaration, PluginDeclaration), AdapterError> {
    if !request
        .default_tool_signals()
        .contains(&SignalKind::Coverage)
    {
        return Ok((None, None));
    }
    let kover = find_plugin(scripts, crate::tooling::KOVER.plugin_ids())?;
    let jacoco = find_jacoco(scripts)?;
    let has_kover = kover.is_some() || declares_plugin(scripts, "org.jetbrains.kotlinx.kover");
    let has_jacoco = jacoco.is_some() || declares_plugin(scripts, "jacoco");
    if has_kover && has_jacoco {
        return Err(error(
            "both Kover and JaCoCo are declared; select one coverage provider",
        ));
    }
    let coverage = crate::tooling::coverage_baseline(if has_jacoco {
        Some("jacoco")
    } else if has_kover {
        Some("kover")
    } else {
        None
    })
    .map_err(error)?;
    plan.tools.retain(|t| {
        !matches!(t.tool.as_str(), "kover" | "jacoco") || t.tool == coverage.catalog_name
    });
    Ok((kover, jacoco))
}

fn declaration(
    tool: &str,
    scripts: &[(String, String)],
    kover: &PluginDeclaration,
    jacoco: &PluginDeclaration,
) -> Result<PluginDeclaration, AdapterError> {
    match tool {
        "kover" => Ok(kover.clone()),
        "jacoco" => Ok(jacoco.clone()),
        "detekt" => find_plugin(scripts, crate::tooling::DETEKT.plugin_ids()),
        "pitest" => find_plugin(scripts, crate::tooling::PITEST.plugin_ids()),
        _ => Ok(None),
    }
}

fn declares_plugin(scripts: &[(String, String)], id: &str) -> bool {
    let escaped = regex::escape(id);
    let pattern = regex::Regex::new(&format!(
        r#"id\(\s*["']{escaped}["']\s*\)|id\s+["']{escaped}["']|(?m)^\s*{escaped}\s*$"#
    ))
    .expect("escaped plugin pattern");
    scripts.iter().any(|(_, content)| pattern.is_match(content))
}
