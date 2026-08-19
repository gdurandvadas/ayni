//! Repository entries omitted from managed workspace copies and source provenance.
//!
//! These names describe generated or tool-owned state rather than checkout source.
//! Keeping the list here ensures provenance and managed execution operate on the
//! same logical workspace.

pub const GENERATED_WORKSPACE_ENTRY_NAMES: &[&str] = &[
    ".ayni",
    ".git",
    ".gradle",
    ".svelte-kit",
    ".venv",
    "__pycache__",
    "build",
    "coverage",
    "node_modules",
    "target",
];

#[must_use]
pub fn is_generated_workspace_entry(name: &str) -> bool {
    GENERATED_WORKSPACE_ENTRY_NAMES.contains(&name)
}

#[cfg(test)]
mod tests {
    use super::{GENERATED_WORKSPACE_ENTRY_NAMES, is_generated_workspace_entry};

    #[test]
    fn generated_workspace_entries_are_sorted_and_unique() {
        assert!(
            GENERATED_WORKSPACE_ENTRY_NAMES
                .windows(2)
                .all(|names| names[0] < names[1])
        );
        assert!(is_generated_workspace_entry("node_modules"));
        assert!(!is_generated_workspace_entry("src"));
    }
}
