use crate::application::{EnvShowOperation, OutputFormat, ToolsReconcileOperation};
use crate::application_error::{ApplicationError, render_error};
use ayni_adapters_common::environment::validate_environment_target_containment;
use ayni_core::{
    AdapterRegistry, SignalToolOwnership, TargetIdentity, ToolingPlan, ToolingRequest,
};
use serde::Serialize;
use std::process::ExitCode;

#[derive(Serialize)]
struct Projection {
    projection_version: &'static str,
    ownership: SignalToolOwnership,
    reconciliation_required: bool,
    environment_lock: &'static str,
    targets: Vec<ToolingPlan>,
}

pub(crate) fn run(operation: ToolsReconcileOperation, registry: &AdapterRegistry) -> ExitCode {
    match project(&operation, registry) {
        Ok(projection) => {
            if operation.output == OutputFormat::Json {
                match serde_json::to_string_pretty(&projection) {
                    Ok(output) => println!("{output}"),
                    Err(cause) => {
                        return render_error(ApplicationError::execution(cause.to_string()));
                    }
                }
            } else {
                render(&projection);
            }
            if operation.check && projection.reconciliation_required {
                ExitCode::from(1)
            } else {
                ExitCode::SUCCESS
            }
        }
        Err(cause) => render_error(cause),
    }
}
fn project(
    operation: &ToolsReconcileOperation,
    registry: &AdapterRegistry,
) -> Result<Projection, ApplicationError> {
    let context = EnvShowOperation {
        config: operation.config.clone(),
        repo_root: operation.repo_root.clone(),
        output: operation.output,
    };
    let (repo, _, config, policy) = crate::environment::load_context(&context)?;
    let mut targets = Vec::new();
    let mut identities = std::collections::BTreeSet::new();
    for language in policy
        .enabled_languages()
        .map_err(ApplicationError::input)?
    {
        for root in policy.roots_for(language) {
            identities.insert(
                TargetIdentity::new(language, root)
                    .map_err(|e| ApplicationError::input(e.to_string()))?,
            );
        }
    }
    for identity in identities {
        validate_environment_target_containment(&repo, &identity)
            .map_err(|e| ApplicationError::input(e.to_string()))?;
        let adapter = registry
            .adapters()
            .iter()
            .find(|a| a.language() == identity.language)
            .ok_or_else(|| ApplicationError::input("configured adapter unavailable"))?;
        let enabled = policy.enabled_signals();
        let defaults = enabled
            .iter()
            .copied()
            .filter(|s| policy.tool_override_for(identity.language, *s).is_none())
            .collect::<Vec<_>>();
        let request = ToolingRequest::new(
            repo.clone(),
            identity.clone(),
            enabled,
            policy.environment.signal_tools.ownership,
            defaults,
        )
        .map_err(|e| ApplicationError::input(e.to_string()))?;
        let plan = adapter.plan_tooling(&request).unwrap_or_else(|cause| {
            let mut plan = ToolingPlan::empty(identity);
            plan.conflicts
                .push(ayni_adapters_common::tooling::diagnostic(
                    "tooling.inspection_blocked",
                    cause.to_string(),
                    None,
                ));
            plan
        });
        targets.push(plan);
    }
    let required = targets.iter().any(|p| !p.conflicts.is_empty());
    let environment_lock = lock_status(&repo, &config, required);
    Ok(Projection {
        projection_version: "0.1.0",
        ownership: policy.environment.signal_tools.ownership,
        reconciliation_required: required || matches!(environment_lock, "stale" | "invalid"),
        environment_lock,
        targets,
    })
}
fn lock_status(repo: &std::path::Path, config: &[u8], changes: bool) -> &'static str {
    if !repo.join(".ayni.lock").exists() {
        return "absent";
    }
    let Ok(lock) = ayni_environment::read_lock(repo) else {
        return "invalid";
    };
    if lock.repository().contract_digest != ayni_core::sha256_fingerprint(config) {
        return "stale";
    }
    for target in lock.targets() {
        for input in &target.dependency_locks {
            let Ok(Some(bytes)) = ayni_adapters_common::repository::read_optional_contained_bytes(
                repo,
                &repo.join(&input.path),
            ) else {
                return "stale";
            };
            if ayni_core::sha256_fingerprint(&bytes) != input.digest {
                return "stale";
            }
        }
    }
    if changes {
        "refresh_after_reconciliation"
    } else {
        "recorded_inputs_match"
    }
}
fn render(projection: &Projection) {
    println!(
        "Signal tooling reconciliation (preview {})",
        projection.projection_version
    );
    println!(
        "Ownership: {:?}\nEnvironment lock: {}",
        projection.ownership, projection.environment_lock
    );
    for plan in &projection.targets {
        println!(
            "\n{}:{} (owner {})",
            plan.target.language, plan.target.root, plan.owner_root
        );
        for tool in &plan.tools {
            println!(
                "  {}: baseline {:?}; declaration {}; resolution {}",
                tool.tool,
                tool.baseline,
                tool.current_declaration.as_deref().unwrap_or("none"),
                tool.current_resolution
                    .as_deref()
                    .unwrap_or("not inspected")
            );
            if let Some(path) = &tool.declaration_path {
                println!("    {path}");
            }
        }
        for finding in plan.conflicts.iter().chain(&plan.warnings) {
            println!("  [{}] {}", finding.code, finding.message);
        }
    }
    println!(
        "\nReconciliation required: {}",
        projection.reconciliation_required
    );
}
