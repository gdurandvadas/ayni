use crate::tooling::*;
use crate::{PreparationCommand, SignalToolOwnership, VersionRequirement};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) fn validate_plan(
    plan: &mut ToolingPlan,
    request: &ToolingRequest,
) -> Result<(), String> {
    validate_identity_and_ownership(plan, request)?;
    validate_tools(&mut plan.tools, request)?;
    let inputs = validate_inputs(&mut plan.inputs)?;
    let outputs = validate_outputs(&mut plan.outputs, &inputs)?;
    let mut produced = validate_edits(&mut plan.edits, &outputs)?;
    for command in &mut plan.commands {
        validate_command(command, request.target().language, &outputs)?;
        produced.extend(command.outputs.iter().cloned());
    }
    if produced.len() != outputs.len() {
        return Err("every tooling output must have an edit or command producer".into());
    }
    validate_diagnostics(plan)?;
    plan.tools.sort_by(|a, b| a.tool.cmp(&b.tool));
    plan.inputs.sort_by(|a, b| a.path.cmp(&b.path));
    plan.outputs.sort_by(|a, b| a.path.cmp(&b.path));
    plan.edits.sort_by(|a, b| a.path.cmp(&b.path));
    // Command order is semantic and must never be sorted.
    Ok(())
}

fn validate_identity_and_ownership(
    plan: &mut ToolingPlan,
    request: &ToolingRequest,
) -> Result<(), String> {
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
    if request.ownership() == SignalToolOwnership::Project && has_writes(plan) {
        return Err("project-owned tooling plans must not propose mutations".into());
    }
    if plan.tools.is_empty() && has_writes(plan) {
        return Err("tooling mutations require a declared tool requirement".into());
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

fn has_writes(plan: &ToolingPlan) -> bool {
    !plan.edits.is_empty() || !plan.commands.is_empty() || !plan.outputs.is_empty()
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

fn validate_inputs(inputs: &mut [ToolingInput]) -> Result<BTreeMap<String, String>, String> {
    let mut paths = BTreeMap::new();
    for input in inputs {
        input.path = relative_path(&input.path, true)?;
        digest(&input.digest)?;
        if paths
            .insert(input.path.clone(), input.digest.clone())
            .is_some()
        {
            return Err("duplicate tooling input".into());
        }
    }
    reject_overlapping_files(paths.keys())?;
    Ok(paths)
}

fn validate_outputs(
    outputs: &mut [ToolingOutput],
    inputs: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, ToolingPreimage>, String> {
    let mut paths = BTreeMap::new();
    for output in outputs {
        output.path = relative_path(&output.path, true)?;
        match &output.preimage {
            ToolingPreimage::Absent if inputs.contains_key(&output.path) => {
                return Err("absent output conflicts with an existing input".into());
            }
            ToolingPreimage::Sha256 { digest: expected } => {
                digest(expected)?;
                if inputs.get(&output.path) != Some(expected) {
                    return Err(
                        "existing tooling output requires a matching staged input digest".into(),
                    );
                }
            }
            ToolingPreimage::Absent => {}
        }
        if paths
            .insert(output.path.clone(), output.preimage.clone())
            .is_some()
        {
            return Err("duplicate tooling output".into());
        }
    }
    let all = paths.keys().chain(inputs.keys()).collect::<BTreeSet<_>>();
    reject_overlapping_files(all)?;
    Ok(paths)
}

fn validate_edits(
    edits: &mut [ToolingFileEdit],
    outputs: &BTreeMap<String, ToolingPreimage>,
) -> Result<BTreeSet<String>, String> {
    let mut paths = BTreeSet::new();
    for edit in edits {
        edit.path = relative_path(&edit.path, true)?;
        if outputs.get(&edit.path) != Some(&edit.preimage) {
            return Err("tooling edit must match a declared output and preimage".into());
        }
        if !paths.insert(edit.path.clone()) {
            return Err("duplicate or conflicting tooling edit".into());
        }
    }
    Ok(paths)
}

fn validate_command(
    command: &mut ToolingCommand,
    language: crate::Language,
    outputs: &BTreeMap<String, ToolingPreimage>,
) -> Result<(), String> {
    validate_program(&command.program)?;
    // Reuse preparation's argv/environment rules, but not its mutation contract.
    let checked = PreparationCommand::new(
        language,
        &command.program,
        command.args.clone(),
        &command.cwd,
        command.environment.clone(),
    )
    .map_err(|cause| cause.message)?;
    command.cwd = relative_path(&checked.cwd, false)?;
    if command.outputs.is_empty() {
        return Err("tooling command must declare output files".into());
    }
    let mut seen = BTreeSet::new();
    for path in &mut command.outputs {
        *path = relative_path(path, true)?;
        if !outputs.contains_key(path) || !seen.insert(path.clone()) {
            return Err("command outputs must be unique declared repository files".into());
        }
    }
    command.outputs.sort();
    Ok(())
}

fn validate_program(program: &str) -> Result<(), String> {
    let mut bytes = program.bytes();
    if !bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        || !bytes.all(|byte| byte.is_ascii_alphanumeric() || b"_.+-".contains(&byte))
    {
        return Err("tooling program must be a bare ASCII executable name".into());
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
        return Err("tooling outputs and inputs must name files".into());
    }
    Ok(parts.join("/"))
}

fn reject_overlapping_files<'a>(paths: impl IntoIterator<Item = &'a String>) -> Result<(), String> {
    let paths = paths
        .into_iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    for path in &paths {
        let mut parent = *path;
        while let Some((prefix, _)) = parent.rsplit_once('/') {
            if paths.contains(prefix) {
                return Err("tooling file paths overlap as file and directory".into());
            }
            parent = prefix;
        }
    }
    Ok(())
}

fn digest(value: &str) -> Result<(), String> {
    let hex = value
        .strip_prefix("sha256:")
        .ok_or("tooling preimage must use sha256")?;
    if hex.len() != 64
        || !hex
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err("tooling preimage must contain a lowercase SHA-256 digest".into());
    }
    Ok(())
}

fn label(value: &str) -> Result<(), String> {
    if value.trim().is_empty() || value.contains('\0') {
        return Err("tooling labels must be nonempty and contain no NUL".into());
    }
    Ok(())
}
