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
        "Discover Ayni commands using standard CLI help:",
        "",
        "- Run `ayni help` to list top-level commands.",
        "- Run `ayni help <command> [subcommand]` to explore nested commands.",
        "- Run `ayni <command> --help` for command-specific options.",
        "",
        "Treat `.ayni.toml` as the authoritative repository quality policy. Run",
        "`ayni contract show` for a concise view of its effective configured signal",
        "contract instead of reading the full policy file.",
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
        "Treat incomplete artifacts as failure, and never loosen `.ayni.toml` merely",
        "to silence a finding.",
        "",
        "Use the full repository analysis as the completion gate:",
        "",
        "```sh",
        "ayni check",
        "```",
        "",
        "A non-zero exit code means at least one signal failed. Read",
        "`.ayni/last/signals.json` for detailed, typed signal results, including",
        "completion state and target accounting. For each finding, rerun its exact",
        "verification command and repair the listed offenders.",
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
