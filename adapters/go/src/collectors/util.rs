use ayni_adapters_common::exec::{self, ExecutionResult};
use ayni_core::RunContext;

pub fn run_tool_for_context(context: &RunContext, tool: &str, args: &[String]) -> ExecutionResult {
    exec::run_command_for_context_structured(context, tool, args)
}
