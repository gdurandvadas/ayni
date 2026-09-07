use crate::environment::{locked, read_toml, requirement_name, uv_workspace_contains};
use ayni_adapters_common::{
    repository::{read_optional_contained_string, repository_relative},
    tooling::{baseline_plan, diagnostic, finish},
};
use ayni_core::{AdapterError, Language, ToolingPlan, ToolingRequest};

pub(crate) fn plan(request: &ToolingRequest) -> Result<ToolingPlan, AdapterError> {
    let mut plan = baseline_plan(
        request,
        crate::catalog::PYTHON_CATALOG,
        crate::tooling::PYTHON_TOOLS,
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
    AdapterError::new(Language::Python, cause.to_string())
}
fn inspect(request: &ToolingRequest, plan: &mut ToolingPlan) -> Result<(), AdapterError> {
    let repo = request.repo_root();
    let target = repo.join(&request.target().root);
    let owner = owner_root(repo, &target)?;
    plan.owner_root = repository_relative(repo, &owner).map_err(error)?;
    let path = owner.join("pyproject.toml");
    let manifest = read_toml(repo, &path)?;
    let target_manifest = read_toml(repo, &target.join("pyproject.toml"))?;
    let lock = read_lock(repo, &owner)?;
    let mut declarations = Vec::new();
    collect_declarations(&manifest, &mut declarations)?;
    let mut members = Vec::new();
    collect_declarations(&target_manifest, &mut members)?;
    fill_requirements(request, plan, &declarations, &members, lock.as_ref())?;
    declarations.extend(members);
    check_resolutions(plan, &declarations);
    Ok(())
}
fn collect_declarations(
    value: &toml::Value,
    out: &mut Vec<(String, String)>,
) -> Result<(), AdapterError> {
    let mut arrays = Vec::new();
    if let Some(v) = value.get("project").and_then(|v| v.get("dependencies")) {
        arrays.push(v);
    }
    for table in [
        value.get("dependency-groups"),
        value
            .get("project")
            .and_then(|v| v.get("optional-dependencies")),
    ]
    .into_iter()
    .flatten()
    {
        arrays.extend(
            table
                .as_table()
                .ok_or_else(|| error("dependency groups must be tables"))?
                .values(),
        );
    }
    if let Some(v) = value
        .get("tool")
        .and_then(|v| v.get("uv"))
        .and_then(|v| v.get("dev-dependencies"))
    {
        arrays.push(v);
    }
    for array in arrays {
        for item in array
            .as_array()
            .ok_or_else(|| error("dependency declarations must be arrays"))?
        {
            let text = item
                .as_str()
                .ok_or_else(|| error("included dependency groups require manual reconciliation"))?;
            out.push((requirement_name(text.into())?, text.into()));
        }
    }
    Ok(())
}

fn owner_root(
    repo: &std::path::Path,
    target: &std::path::Path,
) -> Result<std::path::PathBuf, AdapterError> {
    let mut owner = target.to_path_buf();
    for ancestor in target
        .ancestors()
        .skip(1)
        .take_while(|p| p.starts_with(repo))
    {
        if ancestor.join("pyproject.toml").is_file()
            && uv_workspace_contains(repo, ancestor, target)?
        {
            owner = ancestor.to_path_buf();
            break;
        }
    }
    for marker in ["poetry.lock", "pdm.lock", "Pipfile.lock"] {
        if owner.join(marker).exists() {
            return Err(error("tooling reconciliation requires a uv project"));
        }
    }
    Ok(owner)
}

fn satisfies(declaration: &str, version: &str) -> Result<bool, AdapterError> {
    let expression = declaration
        .trim_start_matches(|c: char| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        .trim();
    if expression.is_empty() {
        return Ok(true);
    }
    let version = crate::environment::parse_python_release(version)?;
    crate::environment::python_requirement_satisfied(version, expression)
}
fn check_resolutions(plan: &mut ToolingPlan, declarations: &[(String, String)]) {
    for tool in &plan.tools {
        let Some(version) = &tool.current_resolution else {
            continue;
        };
        for (_, declaration) in declarations.iter().filter(|(name, _)| name == &tool.tool) {
            let message = match satisfies(declaration, version) {
                Ok(true) => continue,
                Ok(false) => format!("{declaration} does not permit locked version {version}"),
                Err(cause) => {
                    format!("{declaration} requires manual compatibility review: {cause}")
                }
            };
            plan.conflicts.push(diagnostic(
                "tooling.lock_mismatch",
                message,
                tool.declaration_path.clone(),
            ));
        }
    }
}

fn resolved_version(
    lock: Option<&toml::Value>,
    tool: &str,
) -> Result<Option<String>, AdapterError> {
    lock.map(|v| locked(v, tool))
        .transpose()
        .map(|value| value.flatten().map(str::to_owned))
}

fn read_lock(
    repo: &std::path::Path,
    owner: &std::path::Path,
) -> Result<Option<toml::Value>, AdapterError> {
    let lock = read_optional_contained_string(repo, &owner.join("uv.lock"))
        .map_err(error)?
        .map(|s| toml::from_str::<toml::Value>(&s))
        .transpose()
        .map_err(error)?;
    Ok(lock)
}

fn fill_requirements(
    request: &ToolingRequest,
    plan: &mut ToolingPlan,
    owner: &[(String, String)],
    member: &[(String, String)],
    lock: Option<&toml::Value>,
) -> Result<(), AdapterError> {
    for tool in &mut plan.tools {
        let inherited = owner.iter().find(|(name, _)| name == &tool.tool);
        let selected = inherited.or_else(|| member.iter().find(|(name, _)| name == &tool.tool));
        tool.current_declaration = selected.map(|(_, text)| text.clone());
        let root = if inherited.is_some() {
            &plan.owner_root
        } else {
            &request.target().root
        };
        tool.declaration_path = Some(
            repository_relative(
                request.repo_root(),
                &request.repo_root().join(root).join("pyproject.toml"),
            )
            .map_err(error)?,
        );
        tool.current_resolution = resolved_version(lock, &tool.tool)?;
    }
    Ok(())
}
