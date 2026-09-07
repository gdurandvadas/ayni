//! Shared read-only reconciliation mechanics; native metadata stays in adapters.
use ayni_core::{
    AdapterError, CatalogEntry, ManagedToolSpec, ToolBaseline, ToolInstallationScope,
    ToolIntegration, ToolVersionAuthority, ToolingDiagnostic, ToolingPlan, ToolingRequest,
    ToolingRequirement, VersionRequirement, select_managed_tools,
};

pub fn baseline_plan(
    request: &ToolingRequest,
    catalog: &[CatalogEntry],
    specs: &[ManagedToolSpec],
) -> Result<ToolingPlan, AdapterError> {
    let mut plan = ToolingPlan::empty(request.target().clone());
    for (spec, signals) in select_managed_tools(catalog, specs, request.default_tool_signals())
        .map_err(|cause| AdapterError::new(request.target().language, cause))?
    {
        let (scope, authority) = match spec.integration {
            ToolIntegration::Runtime => continue,
            ToolIntegration::ToolchainComponent => (
                ToolInstallationScope::Runtime,
                ToolVersionAuthority::Toolchain,
            ),
            ToolIntegration::Isolated { .. } => (
                ToolInstallationScope::Isolated,
                ToolVersionAuthority::AdapterPinned,
            ),
            _ => (
                ToolInstallationScope::Project,
                ToolVersionAuthority::ProjectLocked,
            ),
        };
        let baseline = match spec.baseline {
            ToolBaseline::Exact(version) => VersionRequirement::exact(version),
            ToolBaseline::Toolchain => VersionRequirement::unresolved("selected runtime toolchain"),
        }
        .map_err(|cause| AdapterError::new(request.target().language, cause.to_string()))?;
        plan.tools.push(ToolingRequirement {
            tool: spec.catalog_name.into(),
            baseline,
            authority,
            scope,
            signals: signals.into_iter().collect(),
            declaration_path: None,
            current_declaration: None,
            current_resolution: None,
        });
    }
    Ok(plan)
}

pub fn diagnostic(
    code: &str,
    message: impl Into<String>,
    path: Option<String>,
) -> ToolingDiagnostic {
    ToolingDiagnostic {
        code: code.into(),
        message: message.into(),
        path,
    }
}

/// Check native declarations and resolutions without enforcing baseline versions.
pub fn finish(plan: &mut ToolingPlan) {
    for tool in &plan.tools {
        if tool.scope != ToolInstallationScope::Project {
            continue;
        }
        let message = if tool.current_declaration.is_none() {
            Some(format!("{} requires a native declaration", tool.tool))
        } else if tool.current_resolution.is_none() {
            Some(format!(
                "{} requires an unambiguous native lock resolution",
                tool.tool
            ))
        } else {
            None
        };
        if let Some(message) = message {
            plan.conflicts.push(diagnostic(
                "tooling.reconciliation_required",
                message,
                tool.declaration_path.clone(),
            ));
        }
    }
}
