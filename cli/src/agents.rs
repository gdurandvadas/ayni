use std::fs;
use std::path::Path;

pub(crate) const MANAGED_BEGIN: &str = "<!-- AYNI:BEGIN -->";
pub(crate) const MANAGED_END: &str = "<!-- AYNI:END -->";

pub(crate) fn sync_impl(repo_root: &str) -> Result<(), String> {
    let path = Path::new(repo_root).join("AGENTS.md");
    let content = if path.exists() {
        fs::read_to_string(&path)
            .map_err(|error| format!("failed to read {}: {error}", path.display()))?
    } else {
        String::new()
    };
    let updated = upsert_managed_block(&content, &managed_block());
    if updated != content {
        fs::write(&path, updated)
            .map_err(|error| format!("failed to write {}: {error}", path.display()))?;
    }
    Ok(())
}

pub(crate) fn managed_block() -> String {
    [
        MANAGED_BEGIN,
        "## Code quality guidance for AI agents",
        "",
        "When modifying this repository:",
        "",
        "- Preserve clear module boundaries.",
        "- Prefer small, testable units.",
        "- Keep CLI, core logic, command execution, and reporting separate.",
        "- Avoid adding network dependencies unless explicitly required.",
        "- Update tests when behavior changes.",
        "",
        "During TDD, run a focused test when the language scope is known:",
        "",
        "```sh",
        "ayni verify test --language <rust|go|node|python|kotlin> [selectors]",
        "```",
        "",
        "Before task completion, run:",
        "",
        "```sh",
        "ayni analyze",
        "```",
        "",
        "A non-zero exit code means at least one signal failed. For typed,",
        "machine-readable schema-v2 results (per-signal offenders, budgets, and deltas),",
        "run `ayni analyze --json` (or `--output json`) or read `.ayni/last/signals.json`",
        "after any analyze run, then repair the listed offenders and re-run",
        "until every row passes.",
        MANAGED_END,
        "",
    ]
    .join("\n")
}

pub(crate) fn upsert_managed_block(existing: &str, managed: &str) -> String {
    let normalized_existing = if existing.is_empty() {
        String::new()
    } else if existing.ends_with('\n') {
        existing.to_string()
    } else {
        format!("{existing}\n")
    };

    let begin = normalized_existing.find(MANAGED_BEGIN);
    let end = normalized_existing.find(MANAGED_END);
    if let (Some(begin_idx), Some(end_idx)) = (begin, end)
        && begin_idx <= end_idx
    {
        let end_exclusive = end_idx + MANAGED_END.len();
        let mut result = String::new();
        result.push_str(&normalized_existing[..begin_idx]);
        result.push_str(managed);
        if end_exclusive < normalized_existing.len() {
            let remainder = normalized_existing[end_exclusive..].trim_start_matches('\n');
            if !remainder.is_empty() {
                result.push_str(remainder);
                if !result.ends_with('\n') {
                    result.push('\n');
                }
            }
        }
        return result;
    }

    if normalized_existing.is_empty() {
        managed.to_string()
    } else {
        format!("{normalized_existing}\n{managed}")
    }
}
