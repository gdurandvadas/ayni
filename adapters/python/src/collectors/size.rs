use ayni_core::size::collect_size;
use ayni_core::{
    Budget, Language, Offenders, RunContext, Scope, SignalKind, SignalResult, SignalRow,
};

pub fn collect(context: &RunContext) -> Result<SignalRow, String> {
    let size_map = context.policy.size_rules_for(Language::Python);
    if size_map.is_empty() {
        return Err(String::from(
            "missing size config: add [python.size] with at least one glob entry to .ayni.toml",
        ));
    }
    let collected = collect_size(
        &context.repo_root,
        &context.workdir,
        size_map,
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
    )?;

    Ok(SignalRow {
        kind: SignalKind::Size,
        language: Language::Python,
        scope: Scope {
            workspace_root: context.scope.workspace_root.clone(),
            path: context.scope.path.clone(),
            package: context.scope.package.clone(),
            file: context.scope.file.clone(),
        },
        pass: collected.result.fail_count == 0,
        result: SignalResult::Size(collected.result),
        budget: Budget::Size(collected.budget),
        offenders: Offenders::Size(collected.offenders),
        delta_vs_previous: None,
        delta_vs_baseline: None,
    })
}
