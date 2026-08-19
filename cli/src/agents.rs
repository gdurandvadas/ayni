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
        "Treat `.ayni.toml` as the authoritative repository quality policy. Run",
        "`ayni contract show` for its validated effective summary, and inspect the",
        "policy itself before proposing a policy change.",
        "",
        "During an edit, use the narrowest supported `ayni verify <signal>`:",
        "",
        "```sh",
        "ayni verify <signal> [selectors]",
        "```",
        "",
        "Use `ayni verify list` to list exact commands from the last repository artifact,",
        "then rerun the exact verification command supplied by a finding. For a change-scoped",
        "loop, run `ayni impact show --base <revision>` and then `ayni impact run`,",
        "copying the same explicit base. Impact success is not repository completion;",
        "run one unscoped `ayni check` at the caller's completion boundary.",
        "",
        "Run quality commands directly; never wrap them in `ayni env run`. Treat `env",
        "shell` and `env run` as intentional advanced access because they mount the",
        "checkout read-write and do not produce normalized quality evidence.",
        "",
        "Treat incomplete or missing required artifacts as failure, and never loosen",
        "`.ayni.toml` merely to silence a finding.",
        "",
        "Use the full repository analysis as the completion gate:",
        "",
        "```sh",
        "ayni check",
        "```",
        "",
        "A non-zero exit code means the quality contract was not satisfied: a signal",
        "may have failed, expected work may be incomplete, or setup may be invalid.",
        "Read `.ayni/last/signals.json` when present for typed completion and target",
        "accounting. For each finding, rerun its exact verification command and repair",
        "the listed offenders.",
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
