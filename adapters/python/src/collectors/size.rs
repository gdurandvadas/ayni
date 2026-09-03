use ayni_adapters_common::size::collect_size_signal;
use ayni_core::{Language, RunContext, SignalRow};

pub fn collect(context: &RunContext) -> Result<SignalRow, String> {
    collect_size_signal(
        context,
        Language::Python,
        &[
            ".venv",
            "venv",
            "env",
            "__pycache__",
            ".pytest_cache",
            ".ruff_cache",
            ".tox",
            ".nox",
            ".git",
            ".ayni",
        ],
    )
}
