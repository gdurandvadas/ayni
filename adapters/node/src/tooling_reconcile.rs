use crate::environment::{
    dependency, node_manifest_inputs, package_manager_owner, read_manifest, workspace_owner,
};
use crate::environment_resolution::{
    ensure_locked_version_matches, locked_tool_version, lockfile_target_root,
    pnpm_locked_tool_version,
};
use ayni_adapters_common::{
    repository::{read_optional_contained_string, repository_relative},
    tooling::{baseline_plan, diagnostic, finish},
};
use ayni_core::{
    AdapterError, Language, SignalToolOwnership, ToolingPlan, ToolingRequest, VersionRequirement,
};

pub(crate) fn plan(request: &ToolingRequest) -> Result<ToolingPlan, AdapterError> {
    let mut plan = baseline_plan(
        request,
        crate::catalog::NODE_CATALOG,
        crate::tooling::NODE_TOOLS,
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
    finish(&mut plan, request);
    Ok(plan)
}
fn error(cause: impl std::fmt::Display) -> AdapterError {
    AdapterError::new(Language::Node, cause.to_string())
}
fn inspect(request: &ToolingRequest, plan: &mut ToolingPlan) -> Result<(), AdapterError> {
    let repo = request.repo_root();
    let target = repo.join(&request.target().root);
    let (owner, manifest, owner_manifest) = manifests(repo, &target)?;
    plan.owner_root = repository_relative(repo, &owner).map_err(error)?;
    let (npm, pnpm) = read_locks(repo, &owner, &owner_manifest)?;
    let relative_target = lockfile_target_root(&plan.owner_root, &request.target().root)?;
    for tool in &mut plan.tools {
        let direct = dependency(&manifest, &tool.tool)?;
        let inherited = dependency(&owner_manifest, &tool.tool)?;
        tool.current_declaration = direct.or(inherited).map(str::to_owned);
        let path = if direct.is_some() {
            target.join("package.json")
        } else {
            owner.join("package.json")
        };
        tool.declaration_path = Some(repository_relative(repo, &path).map_err(error)?);
        tool.current_resolution =
            resolved_version(npm.as_ref(), pnpm.as_deref(), &relative_target, &tool.tool);
        check_resolution(tool, &mut plan.conflicts)?;
    }
    if request.ownership() == SignalToolOwnership::Ayni {
        check_owner(request, plan, &owner, &owner_manifest, &target)?;
    }
    Ok(())
}

fn check_resolution(
    tool: &ayni_core::ToolingRequirement,
    conflicts: &mut Vec<ayni_core::ToolingDiagnostic>,
) -> Result<(), AdapterError> {
    if let (Some(declaration), Some(version)) =
        (&tool.current_declaration, &tool.current_resolution)
        && let Err(cause) = ensure_locked_version_matches(
            &tool.tool,
            version,
            &VersionRequirement::selector(declaration).map_err(error)?,
            "native lock",
        )
    {
        conflicts.push(diagnostic(
            "tooling.lock_mismatch",
            cause.to_string(),
            tool.declaration_path.clone(),
        ));
    }
    Ok(())
}
fn check_owner(
    request: &ToolingRequest,
    plan: &mut ToolingPlan,
    owner: &std::path::Path,
    owner_manifest: &serde_json::Value,
    target: &std::path::Path,
) -> Result<(), AdapterError> {
    let repo = request.repo_root();
    let members = node_manifest_inputs(repo, owner, target)?
        .into_iter()
        .filter(|path| *path != owner.join("package.json"))
        .map(|path| {
            let manifest = read_manifest(repo, &path, true)?.expect("required member");
            Ok((repository_relative(repo, &path).map_err(error)?, manifest))
        })
        .collect::<Result<Vec<_>, AdapterError>>()?;
    for tool in &plan.tools {
        let VersionRequirement::Exact { version: baseline } = &tool.baseline else {
            continue;
        };
        let dev = owner_manifest
            .get("devDependencies")
            .and_then(|v| v.get(&tool.tool))
            .and_then(serde_json::Value::as_str);
        if dev != Some(baseline.as_str()) {
            plan.conflicts.push(diagnostic(
                "tooling.owner_declaration_required",
                format!(
                    "declare {} = {baseline} in governing devDependencies",
                    tool.tool
                ),
                Some(repository_relative(repo, &owner.join("package.json")).map_err(error)?),
            ));
        }
        check_members(tool, baseline, &members, &mut plan.conflicts)?;
    }
    Ok(())
}
fn check_members(
    tool: &ayni_core::ToolingRequirement,
    baseline: &str,
    members: &[(String, serde_json::Value)],
    conflicts: &mut Vec<ayni_core::ToolingDiagnostic>,
) -> Result<(), AdapterError> {
    for (path, member) in members {
        if let Some(declaration) = dependency(member, &tool.tool)?
            && let Err(cause) = ensure_locked_version_matches(
                &tool.tool,
                baseline,
                &VersionRequirement::selector(declaration).map_err(error)?,
                "adapter baseline",
            )
        {
            conflicts.push(diagnostic(
                "tooling.member_constraint",
                cause.to_string(),
                Some(path.clone()),
            ));
        }
    }
    Ok(())
}

fn read_locks(
    repo: &std::path::Path,
    owner: &std::path::Path,
    owner_manifest: &serde_json::Value,
) -> Result<(Option<serde_json::Value>, Option<String>), AdapterError> {
    let npm =
        read_optional_contained_string(repo, &owner.join("package-lock.json")).map_err(error)?;
    let pnpm =
        read_optional_contained_string(repo, &owner.join("pnpm-lock.yaml")).map_err(error)?;
    if npm.is_some() && pnpm.is_some() {
        return Err(error("conflicting npm and pnpm lockfiles"));
    }
    if owner_manifest
        .get("packageManager")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|v| !v.starts_with("npm@") && !v.starts_with("pnpm@"))
    {
        return Err(error("reconciliation supports npm and pnpm only"));
    }
    let npm = npm
        .as_deref()
        .map(serde_json::from_str::<serde_json::Value>)
        .transpose()
        .map_err(error)?;
    Ok((npm, pnpm))
}

fn resolved_version(
    npm: Option<&serde_json::Value>,
    pnpm: Option<&str>,
    target: &str,
    tool: &str,
) -> Option<String> {
    npm.and_then(|lock| locked_tool_version(lock, target, tool))
        .map(str::to_owned)
        .or_else(|| pnpm.and_then(|lock| pnpm_locked_tool_version(lock, target, tool)))
}

fn manifests(
    repo: &std::path::Path,
    target: &std::path::Path,
) -> Result<(std::path::PathBuf, serde_json::Value, serde_json::Value), AdapterError> {
    let manifest =
        read_manifest(repo, &target.join("package.json"), true)?.expect("required manifest");
    let workspace = workspace_owner(repo, target)?;
    let owner = package_manager_owner(repo, target, &manifest, &workspace)?;
    let owner_manifest =
        read_manifest(repo, &owner.join("package.json"), true)?.expect("required manifest");
    Ok((owner, manifest, owner_manifest))
}
