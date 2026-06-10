use ayni_adapters_common::exec;
use ayni_core::RunContext;

pub fn run_tool_for_context(
    context: &RunContext,
    tool: &str,
    args: &[String],
) -> Result<std::process::Output, String> {
    exec::run_command_for_context(context, tool, args)
}
