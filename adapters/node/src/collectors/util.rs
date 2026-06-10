use ayni_adapters_common::exec::run_command_for_context;
use ayni_adapters_common::failure::command_failure_with_classification;
use ayni_core::{CommandFailure, NodePackageManager, RunContext, SignalKind};

pub fn package_manager_for_context(context: &RunContext) -> NodePackageManager {
    NodePackageManager::from_executable(&context.execution.runner)
        .unwrap_or(NodePackageManager::Npm)
}

pub fn run_tool(
    context: &RunContext,
    tool: &str,
    args: &[&str],
) -> Result<std::process::Output, String> {
    let manager = package_manager_for_context(context);
    let (program, argv) = manager.exec_command(tool, args);
    run_command_for_context(context, program.as_str(), &argv).map_err(|error| {
        format!(
            "failed to execute {} {}: {error}",
            manager.executable(),
            tool
        )
    })
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
