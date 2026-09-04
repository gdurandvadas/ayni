use ayni_adapters_common::size::collect_size_signal;
use ayni_core::{Language, RunContext, SignalRow};

pub fn collect(context: &RunContext) -> Result<SignalRow, String> {
    collect_size_signal(context, Language::Node, &["node_modules", ".git", ".ayni"])
}
