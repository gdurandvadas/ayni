use super::*;
use serde::Serialize;

#[derive(Serialize)]
struct PlanEnvelope<'a> {
    schema_version: &'static str,
    execution_mode: &'static str,
    plan: &'a ImpactPlan,
}

pub(super) fn emit_plan(plan: &ImpactPlan, operation: &ImpactOperation) -> Result<(), Error> {
    match operation.output {
        OutputFormat::Json => {
            let envelope = PlanEnvelope {
                schema_version: IMPACT_SCHEMA_VERSION,
                execution_mode: execution_mode_name(effective_execution_mode(
                    operation.execution_mode,
                )),
                plan,
            };
            let serialized = serde_json::to_string_pretty(&envelope).map_err(|error| {
                Error::execution(format!("failed to serialize impact plan: {error}"))
            })?;
            println!("{serialized}");
        }
        OutputFormat::Markdown => {
            print_plan_markdown(plan, effective_execution_mode(operation.execution_mode));
        }
        OutputFormat::Human => {
            print_plan_human(plan, effective_execution_mode(operation.execution_mode));
        }
    }
    Ok(())
}

pub(super) fn emit_artifact(
    artifact: &ImpactArtifact,
    serialized: &str,
    output: OutputFormat,
) -> Result<(), Error> {
    match output {
        OutputFormat::Json => print!("{serialized}"),
        OutputFormat::Markdown => {
            print_plan_markdown(&artifact.plan, mode_from_name(&artifact.execution_mode));
            println!("\n## Impact execution\n");
            println!(
                "- State: `{:?}`\n- Jobs: {}/{}\n- Selected rows passing: {}/{}",
                artifact.execution.state,
                artifact.execution.completed_jobs,
                artifact.execution.planned_jobs,
                artifact.aggregate.passing_rows,
                artifact.rows.len()
            );
        }
        OutputFormat::Human => {
            print_plan_human(&artifact.plan, mode_from_name(&artifact.execution_mode));
            println!("execution: {:?}", artifact.execution.state);
            println!(
                "selected jobs: {}/{} completed; {} passing, {} failing",
                artifact.execution.completed_jobs,
                artifact.execution.planned_jobs,
                artifact.aggregate.passing_rows,
                artifact.aggregate.failing_rows
            );
            for issue in &artifact.execution.issues {
                println!("  issue: {}", issue.message);
            }
        }
    }
    Ok(())
}

fn print_plan_human(plan: &ImpactPlan, mode: ExecutionMode) {
    println!("ayni impact plan");
    println!(
        "base: {} -> {}",
        plan.base.requested.as_deref().unwrap_or("<unknown>"),
        plan.base.revision
    );
    println!(
        "candidate: working tree at {} ({})",
        plan.candidate.revision,
        plan.candidate.fingerprint.as_deref().unwrap_or("<unknown>")
    );
    println!("execution mode: {}", execution_mode_name(mode));
    println!("changes: {}", plan.changes.len());
    for change in &plan.changes {
        println!("  {:?} {}", change.kind, change.path);
    }
    println!("selected checks: {}", plan.selected_checks.len());
    for check in &plan.selected_checks {
        let scope = check
            .package
            .as_deref()
            .map(|value| format!("package {value}"))
            .or_else(|| check.file.as_deref().map(|value| format!("file {value}")))
            .unwrap_or_else(|| String::from("configured root"));
        println!(
            "  {}:{} {} ({scope}, {:?})",
            check.language,
            check.configured_root,
            signal_kind_slug(check.signal),
            check.confidence
        );
        for reason in &check.reasons {
            println!("    because {:?}: {}", reason.kind, reason.detail);
        }
    }
    for uncertainty in &plan.uncertainties {
        println!(
            "  uncertainty {:?}: {}",
            uncertainty.kind, uncertainty.detail
        );
    }
    println!("impact evidence is not repository completion; run `ayni check`");
}

fn print_plan_markdown(plan: &ImpactPlan, mode: ExecutionMode) {
    println!("# Ayni impact plan\n");
    println!(
        "> Impact evidence covers only the selected change. Run `ayni check` for repository completion.\n"
    );
    println!("- Base: `{}`", plan.base.revision);
    println!("- Candidate HEAD: `{}`", plan.candidate.revision);
    println!("- Execution mode: `{}`", execution_mode_name(mode));
    println!("- Changed paths: {}", plan.changes.len());
    println!("- Selected checks: {}\n", plan.selected_checks.len());
    for check in &plan.selected_checks {
        println!(
            "- `{}` `{}` `{}`",
            check.language,
            check.configured_root,
            signal_kind_slug(check.signal)
        );
        for reason in &check.reasons {
            println!("  - {:?}: {}", reason.kind, reason.detail);
        }
    }
}

pub(super) fn effective_execution_mode(requested: ExecutionMode) -> ExecutionMode {
    effective_execution_mode_when(requested, managed_execution_active())
}

pub(super) fn effective_execution_mode_when(
    requested: ExecutionMode,
    managed: bool,
) -> ExecutionMode {
    if managed {
        ExecutionMode::Managed
    } else {
        requested
    }
}

pub(super) fn execution_mode_name(mode: ExecutionMode) -> &'static str {
    match mode {
        ExecutionMode::Managed => "managed",
        ExecutionMode::Host => "host",
    }
}

fn mode_from_name(name: &str) -> ExecutionMode {
    if name == "managed" {
        ExecutionMode::Managed
    } else {
        ExecutionMode::Host
    }
}
