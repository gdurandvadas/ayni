use crate::VersionRequirement;
use crate::tooling::*;
use std::collections::BTreeSet;

pub(crate) fn validate_plan(
    plan: &mut ToolingPlan,
    request: &ToolingRequest,
) -> Result<(), String> {
    validate_identity(plan, request)?;
    validate_tools(&mut plan.tools, request)?;
    validate_diagnostics(plan)?;
    plan.tools.sort_by(|a, b| a.tool.cmp(&b.tool));
    Ok(())
}

fn validate_identity(plan: &mut ToolingPlan, request: &ToolingRequest) -> Result<(), String> {
    if plan.target != *request.target() {
        return Err("tooling plan target does not match request".into());
    }
    plan.owner_root = relative_path(&plan.owner_root, false)?;
    if plan.owner_root != "."
        && plan.target.root != plan.owner_root
        && !plan
            .target
            .root
            .starts_with(&format!("{}/", plan.owner_root))
    {
        return Err("tooling owner must contain the requested target".into());
    }
    Ok(())
}

fn validate_diagnostics(plan: &mut ToolingPlan) -> Result<(), String> {
    for diagnostic in plan.warnings.iter_mut().chain(&mut plan.conflicts) {
        label(&diagnostic.code)?;
        label(&diagnostic.message)?;
        if let Some(path) = &mut diagnostic.path {
            *path = relative_path(path, true)?;
        }
    }
    plan.warnings.sort();
    plan.warnings.dedup();
    plan.conflicts.sort();
    plan.conflicts.dedup();
    Ok(())
}

fn validate_tools(
    tools: &mut [ToolingRequirement],
    request: &ToolingRequest,
) -> Result<(), String> {
    let mut names = BTreeSet::new();
    for tool in tools {
        label(&tool.tool)?;
        if !names.insert(tool.tool.clone()) {
            return Err("duplicate tooling requirement".into());
        }
        if tool.signals.is_empty() || !tool.signals.is_subset(request.default_tool_signals()) {
            return Err("tool requirements must belong to enabled default-tool signals".into());
        }
        crate::environment::normalize_version_requirement(&mut tool.baseline)
            .map_err(|cause| cause.to_string())?;
        tool.authority.validate(
            tool.scope,
            matches!(tool.baseline, VersionRequirement::Exact { .. }),
        )?;
        if let Some(path) = &mut tool.declaration_path {
            *path = relative_path(path, true)?;
        }
    }
    Ok(())
}

fn validate_path_spelling(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.starts_with('/')
        || value.contains(['\\', ':'])
        || value.chars().any(char::is_control)
    {
        return Err("tooling path must be a portable repository-relative path".into());
    }
    Ok(())
}

fn relative_path(value: &str, file: bool) -> Result<String, String> {
    validate_path_spelling(value)?;
    let mut parts = Vec::new();
    for part in value.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                return Err(
                    "tooling path escapes scope or names protected repository state".into(),
                );
            }
            _ => {
                if matches!(
                    part.to_ascii_lowercase().as_str(),
                    ".git" | ".ayni" | ".ayni.toml" | ".ayni.lock"
                ) {
                    return Err("tooling path names protected repository state".into());
                }
                parts.push(part);
            }
        }
    }
    if parts.is_empty() {
        if file {
            return Err("tooling file path must not name a directory root".into());
        }
        return Ok(".".into());
    }
    if file && (value.ends_with('/') || value.ends_with("/.")) {
        return Err("tooling declaration paths must name files".into());
    }
    Ok(parts.join("/"))
}

fn label(value: &str) -> Result<(), String> {
    if value.trim().is_empty() || value.contains('\0') {
        return Err("tooling labels must be nonempty and contain no NUL".into());
    }
    Ok(())
}
