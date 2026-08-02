use crate::package_manager::PackageManager;
use ayni_adapters_common::collector::CollectorError;
use ayni_adapters_common::exec::run_command_for_context_structured;
use ayni_adapters_common::failure::command_failure_with_classification;
use ayni_core::{CommandFailure, RunContext, SignalKind};

pub(crate) fn package_manager_for_context(context: &RunContext) -> PackageManager {
    PackageManager::from_runner(&context.execution.runner).unwrap_or(PackageManager::Npm)
}

pub fn tool_command(context: &RunContext, tool: &str, args: &[&str]) -> (String, Vec<String>) {
    let manager = package_manager_for_context(context);
    let (_, argv) = manager.exec_command(tool, args);
    (context.execution.runner.clone(), argv)
}

pub fn run_tool(
    context: &RunContext,
    tool: &str,
    args: &[&str],
) -> Result<std::process::Output, CollectorError> {
    let (program, argv) = tool_command(context, tool, args);
    Ok(run_command_for_context_structured(
        context, &program, &argv,
    )?)
}

pub fn command_failure_from_output(
    context: &RunContext,
    kind: SignalKind,
    program: &str,
    args: &[String],
    output: &std::process::Output,
) -> CommandFailure {
    command_failure_with_classification(
        context,
        kind,
        program,
        args,
        output,
        failure_classification(output),
    )
}

fn failure_classification(output: &std::process::Output) -> &'static str {
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    if text.contains("Cannot find module") || text.contains("ERR_MODULE_NOT_FOUND") {
        "import_error"
    } else if text.contains("No test files found") {
        "no_tests"
    } else {
        "command_error"
    }
}
