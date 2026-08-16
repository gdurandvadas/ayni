//! Shared Node workspace pattern parsing and membership rules.

use glob::{MatchOptions, Pattern};
use serde_json::Value;
use std::path::Path;

pub(crate) struct WorkspacePatterns {
    patterns: Vec<WorkspacePattern>,
}

struct WorkspacePattern {
    excluded: bool,
    raw: String,
    pattern: Pattern,
}

impl WorkspacePatterns {
    pub(crate) fn parse(manifest: &Value, path: &Path) -> Result<Self, String> {
        let Some(workspaces) = manifest.get("workspaces") else {
            return Ok(Self {
                patterns: Vec::new(),
            });
        };
        let values = if let Some(array) = workspaces.as_array() {
            array
        } else if let Some(array) = workspaces.get("packages").and_then(Value::as_array) {
            array
        } else {
            return Err(format!(
                "{} workspaces must be an array or an object with a packages array",
                path.display()
            ));
        };
        let patterns = values
            .iter()
            .map(|value| {
                let raw = value.as_str().ok_or_else(|| {
                    format!(
                        "{} workspaces must contain only string patterns",
                        path.display()
                    )
                })?;
                let (excluded, pattern) = raw
                    .strip_prefix('!')
                    .map_or((false, raw), |value| (true, value));
                let normalized = pattern.trim_end_matches('/').to_owned();
                Ok(WorkspacePattern {
                    excluded,
                    pattern: Pattern::new(&normalized).map_err(|error| {
                        format!("invalid Node workspace pattern {raw}: {error}")
                    })?,
                    raw: normalized,
                })
            })
            .collect::<Result<_, String>>()?;
        Ok(Self { patterns })
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.patterns.is_empty()
    }

    pub(crate) fn matches(&self, target: &str) -> bool {
        let mut included = false;
        for pattern in &self.patterns {
            if workspace_pattern_matches(pattern, target) {
                if pattern.excluded {
                    return false;
                }
                included = true;
            }
        }
        included
    }
}

fn workspace_pattern_matches(pattern: &WorkspacePattern, target: &str) -> bool {
    pattern.pattern.matches_with(target, match_options())
        || pattern
            .raw
            .strip_suffix("/**")
            .is_some_and(|base| target == base)
}

fn match_options() -> MatchOptions {
    MatchOptions {
        case_sensitive: true,
        require_literal_separator: true,
        require_literal_leading_dot: false,
    }
}

#[cfg(test)]
mod tests {
    use super::WorkspacePatterns;
    use serde_json::json;
    use std::path::Path;

    #[test]
    fn negated_patterns_exclude_members_selected_by_an_include() {
        let patterns = WorkspacePatterns::parse(
            &json!({"workspaces": ["packages/**", "!packages/excluded/**"]}),
            Path::new("package.json"),
        )
        .expect("patterns");

        assert!(patterns.matches("packages/api"));
        assert!(!patterns.matches("packages/excluded"));
        assert!(!patterns.matches("packages/excluded/nested"));
    }
}
