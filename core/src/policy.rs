use crate::language::Language;
use crate::signal::SignalKind;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::str::FromStr;

pub const AYNI_POLICY_FILE: &str = ".ayni.toml";

/// Line-count budget for files matching a single glob pattern.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct SizeThreshold {
    pub warn: u64,
    pub fail: u64,
    /// Glob patterns to exclude from this rule.
    pub exclude: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ToolCommandOverride {
    pub command: String,
    pub args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct LanguageToolingOverrides {
    pub test: Option<ToolCommandOverride>,
    pub coverage: Option<ToolCommandOverride>,
    pub mutation: Option<ToolCommandOverride>,
}

/// Per-language tooling thresholds. Maps from TOML tables like `[rust]`.
///
/// Every sub-section is optional; missing sections mean "not configured".
/// `size` is a glob-keyed map: `[rust.size]` with `"*.rs" = { warn = 400, fail = 700 }`.
#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct LanguageTooling {
    #[serde(default = "default_language_roots")]
    pub roots: Vec<String>,
    pub complexity: Option<ComplexityPolicy>,
    pub coverage: Option<CoveragePolicy>,
    pub deps: Option<DepsPolicy>,
    pub tooling: LanguageToolingOverrides,
    /// Glob → threshold. TOML: `[rust.size]` / `[node.size]` etc.
    pub size: BTreeMap<String, SizeThreshold>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct AyniPolicy {
    pub checks: PolicyChecks,
    pub languages: LanguageSelection,
    pub report: ReportPolicy,
    pub concurrency: ConcurrencyPolicy,
    #[serde(default)]
    pub rust: LanguageTooling,
    #[serde(default)]
    pub go: LanguageTooling,
    #[serde(default)]
    pub node: LanguageTooling,
    #[serde(flatten)]
    pub extras: BTreeMap<String, toml::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ReportPolicy {
    pub offenders_limit: usize,
}

impl Default for ReportPolicy {
    fn default() -> Self {
        Self {
            offenders_limit: usize::MAX,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ConcurrencyPolicy {
    /// When false, `amount` is a global limit across all analyze targets.
    /// When true, each language gets its own `amount`-sized worker pool.
    pub per_language: bool,
    /// Maximum number of analyze targets to run concurrently.
    pub amount: usize,
}

impl Default for ConcurrencyPolicy {
    fn default() -> Self {
        Self {
            per_language: false,
            amount: 1,
        }
    }
}

impl AyniPolicy {
    pub fn load(repo_root: &Path) -> Result<Self, String> {
        let path = repo_root.join(AYNI_POLICY_FILE);
        Self::load_from_path(&path)
    }

    pub fn load_from_path(config_path: &Path) -> Result<Self, String> {
        let content = fs::read_to_string(config_path)
            .map_err(|error| format!("failed to read {}: {error}", config_path.display()))?;
        let mut policy = toml::from_str::<Self>(&content)
            .map_err(|error| format!("failed to parse {}: {error}", config_path.display()))?;
        policy
            .normalize_and_validate()
            .map_err(|error| format!("failed to parse {}: {error}", config_path.display()))?;
        Ok(policy)
    }

    /// Whether this language's adapter should run.
    #[must_use]
    pub fn language_allowed(&self, language: Language) -> bool {
        self.languages
            .enabled
            .iter()
            .any(|value| value == language.as_str())
    }

    pub fn enabled_languages(&self) -> Result<Vec<Language>, String> {
        let mut out = Vec::with_capacity(self.languages.enabled.len());
        for value in &self.languages.enabled {
            out.push(
                Language::from_str(value).map_err(|_| {
                    format!(
                        "languages.enabled contains unsupported language '{value}'; expected rust, go, or node"
                    )
                })?,
            );
        }
        Ok(out)
    }

    #[must_use]
    pub fn language_tooling(&self, language: Language) -> &LanguageTooling {
        match language {
            Language::Rust => &self.rust,
            Language::Go => &self.go,
            Language::Node => &self.node,
        }
    }

    /// Effective size map for a language: the language-scoped `[<lang>.size]` map.
    #[must_use]
    pub fn size_rules_for(&self, language: Language) -> &BTreeMap<String, SizeThreshold> {
        &self.language_tooling(language).size
    }

    #[must_use]
    pub fn roots_for(&self, language: Language) -> &[String] {
        &self.language_tooling(language).roots
    }

    #[must_use]
    pub fn tool_override_for(
        &self,
        language: Language,
        kind: SignalKind,
    ) -> Option<&ToolCommandOverride> {
        let tooling = &self.language_tooling(language).tooling;
        match kind {
            SignalKind::Test => tooling.test.as_ref(),
            SignalKind::Coverage => tooling.coverage.as_ref(),
            SignalKind::Mutation => tooling.mutation.as_ref(),
            SignalKind::Size | SignalKind::Complexity | SignalKind::Deps => None,
        }
    }

    fn normalize_and_validate(&mut self) -> Result<(), String> {
        if self.languages.enabled.is_empty() {
            return Err(String::from(
                "languages.enabled must be an explicit non-empty list (for example: [\"rust\"])",
            ));
        }
        for value in &self.languages.enabled {
            if value == "auto" {
                return Err(String::from(
                    "languages.enabled value 'auto' is not supported in v0; use an explicit list like [\"rust\"]",
                ));
            }
            Language::from_str(value).map_err(|_| {
                format!(
                    "languages.enabled contains unsupported language '{value}'; expected rust, go, or node"
                )
            })?;
        }
        self.rust.roots = normalize_roots("rust", &self.rust.roots)?;
        self.go.roots = normalize_roots("go", &self.go.roots)?;
        self.node.roots = normalize_roots("node", &self.node.roots)?;
        if self.concurrency.amount == 0 {
            return Err(String::from("concurrency.amount must be at least 1"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PolicyChecks {
    pub test: bool,
    pub coverage: bool,
    pub size: bool,
    pub complexity: bool,
    pub deps: bool,
    pub mutation: bool,
}

impl Default for PolicyChecks {
    fn default() -> Self {
        Self {
            test: true,
            coverage: true,
            size: true,
            complexity: true,
            deps: true,
            mutation: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LanguageSelection {
    pub enabled: Vec<String>,
}

impl Default for LanguageSelection {
    fn default() -> Self {
        Self {
            enabled: vec![String::from("rust")],
        }
    }
}

fn default_language_roots() -> Vec<String> {
    vec![String::from(".")]
}

fn normalize_roots(language: &str, roots: &[String]) -> Result<Vec<String>, String> {
    let source = if roots.is_empty() {
        default_language_roots()
    } else {
        roots.to_vec()
    };
    let mut normalized = Vec::new();
    for root in source {
        let value = normalize_root_entry(language, &root)?;
        if !normalized.iter().any(|existing| existing == &value) {
            normalized.push(value);
        }
    }
    Ok(normalized)
}

fn normalize_root_entry(language: &str, value: &str) -> Result<String, String> {
    let mut normalized = value.trim().replace('\\', "/");
    while normalized.ends_with('/') {
        normalized.pop();
    }
    if normalized.is_empty() {
        return Ok(String::from("."));
    }
    if normalized.starts_with('/') {
        return Err(format!(
            "{language}.roots entry '{value}' must be repo-relative, not absolute"
        ));
    }
    if normalized == ".." || normalized.starts_with("../") || normalized.contains("/../") {
        return Err(format!(
            "{language}.roots entry '{value}' must stay within repository root"
        ));
    }
    if normalized.len() >= 3 {
        let bytes = normalized.as_bytes();
        if bytes[1] == b':' && bytes[2] == b'/' && bytes[0].is_ascii_alphabetic() {
            return Err(format!(
                "{language}.roots entry '{value}' must be repo-relative, not absolute"
            ));
        }
    }
    Ok(normalized)
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThresholdFloat {
    pub warn: f64,
    pub fail: f64,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ComplexityPolicy {
    pub fn_cyclomatic: Option<ThresholdFloat>,
    pub fn_cognitive: Option<ThresholdFloat>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct CoveragePolicy {
    pub line_percent: Option<ThresholdFloat>,
    pub branch_percent: Option<ThresholdFloat>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct DepsPolicy {
    pub forbidden: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThresholdInt {
    pub warn: u64,
    pub fail: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::Language;

    #[test]
    fn empty_rust_table_parses() {
        let document = r#"
[checks]
test = true
coverage = false
size = false
complexity = false
deps = false
mutation = false

[languages]
enabled = ["rust"]

[rust]
"#;
        let policy: AyniPolicy = toml::from_str(document).expect("parse");
        assert!(policy.rust.complexity.is_none());
        assert!(policy.rust.size.is_empty());
    }

    #[test]
    fn rust_size_map_parses() {
        let document = r#"
[checks]
test = false
coverage = false
size = true
complexity = false
deps = false
mutation = false

[languages]
enabled = ["rust"]

[rust.size]
"*.rs" = { warn = 400, fail = 700 }
"#;
        let policy: AyniPolicy = toml::from_str(document).expect("parse");
        let size = policy.size_rules_for(Language::Rust);
        let rule = size.get("*.rs").expect("*.rs rule");
        assert_eq!(rule.warn, 400);
        assert_eq!(rule.fail, 700);
    }

    #[test]
    fn rust_complexity_parses() {
        let document = r#"
[checks]
test = true
coverage = false
size = false
complexity = true
deps = false
mutation = false

[languages]
enabled = ["rust"]

[rust.complexity]
fn_cyclomatic = { warn = 10.0, fail = 20.0 }
"#;
        let policy: AyniPolicy = toml::from_str(document).expect("parse");
        let c = policy
            .rust
            .complexity
            .as_ref()
            .expect("complexity")
            .fn_cyclomatic
            .expect("cyclomatic");
        assert_eq!(c.warn, 10.0);
        assert_eq!(c.fail, 20.0);
    }

    #[test]
    fn language_tooling_overrides_parse() {
        let document = r#"
[checks]
test = true
coverage = true
size = false
complexity = false
deps = false
mutation = true

[languages]
enabled = ["rust", "go", "node"]

[rust.tooling.test]
command = "cargo"
args = ["nextest", "run"]

[go.tooling.coverage]
command = "gotestsum"
args = ["--", "./..."]

[node.tooling.mutation]
command = "pnpm"
args = ["exec", "stryker", "run"]
"#;
        let policy: AyniPolicy = toml::from_str(document).expect("parse");
        let rust_test = policy
            .tool_override_for(Language::Rust, SignalKind::Test)
            .expect("rust test override");
        assert_eq!(rust_test.command, "cargo");
        assert_eq!(rust_test.args, vec!["nextest", "run"]);

        let go_coverage = policy
            .tool_override_for(Language::Go, SignalKind::Coverage)
            .expect("go coverage override");
        assert_eq!(go_coverage.command, "gotestsum");

        let node_mutation = policy
            .tool_override_for(Language::Node, SignalKind::Mutation)
            .expect("node mutation override");
        assert_eq!(node_mutation.command, "pnpm");
    }

    #[test]
    fn report_policy_defaults_when_omitted() {
        let document = r#"
[checks]
test = true
coverage = true
size = true
complexity = true
deps = true
mutation = false

[languages]
enabled = ["rust"]
"#;
        let policy: AyniPolicy = toml::from_str(document).expect("parse");
        assert_eq!(policy.report.offenders_limit, usize::MAX);
        assert_eq!(policy.concurrency, ConcurrencyPolicy::default());
    }

    #[test]
    fn report_policy_parses_explicit_offenders_limit() {
        let document = r#"
[checks]
test = true
coverage = true
size = true
complexity = true
deps = true
mutation = false

[languages]
enabled = ["rust"]

[report]
offenders_limit = 4
"#;
        let policy: AyniPolicy = toml::from_str(document).expect("parse");
        assert_eq!(policy.report.offenders_limit, 4);
    }

    #[test]
    fn rust_size_exclude_parses() {
        let document = r#"
[checks]
test = false
coverage = false
size = true
complexity = false
deps = false
mutation = false

[languages]
enabled = ["rust"]

[rust.size]
"*.rs" = { warn = 400, fail = 700, exclude = ["target/**", "node_modules/**"] }
"#;
        let policy: AyniPolicy = toml::from_str(document).expect("parse");
        let rule = policy
            .size_rules_for(Language::Rust)
            .get("*.rs")
            .expect("rule");
        assert_eq!(rule.exclude, vec!["target/**", "node_modules/**"]);
    }

    #[test]
    fn multi_language_size_maps_are_independent() {
        let document = r#"
[checks]
test = false
coverage = false
size = true
complexity = false
deps = false
mutation = false

[languages]
enabled = ["rust", "node"]

[rust.size]
"*.rs" = { warn = 400, fail = 700 }

[node.size]
"**/*.ts" = { warn = 300, fail = 600 }
"**/*.tsx" = { warn = 200, fail = 400 }
"#;
        let policy: AyniPolicy = toml::from_str(document).expect("parse");
        assert_eq!(policy.size_rules_for(Language::Rust).len(), 1);
        assert_eq!(policy.size_rules_for(Language::Node).len(), 2);
        assert!(policy.size_rules_for(Language::Go).is_empty());
    }

    #[test]
    fn default_roots_to_current_directory() {
        let document = r#"
[checks]
test = true
coverage = false
size = false
complexity = false
deps = false
mutation = false

[languages]
enabled = ["rust"]
"#;
        let mut policy: AyniPolicy = toml::from_str(document).expect("parse");
        policy.normalize_and_validate().expect("valid");
        assert_eq!(policy.roots_for(Language::Rust), ["."]);
        assert_eq!(policy.roots_for(Language::Go), ["."]);
        assert_eq!(policy.roots_for(Language::Node), ["."]);
    }

    #[test]
    fn rejects_auto_language_selection() {
        let document = r#"
[checks]
test = true
coverage = true
size = true
complexity = true
deps = true
mutation = false

[languages]
enabled = ["auto"]
"#;
        let mut policy: AyniPolicy = toml::from_str(document).expect("parse");
        let error = policy.normalize_and_validate().expect_err("must fail");
        assert!(error.contains("not supported in v0"));
    }

    #[test]
    fn normalizes_roots_entries() {
        let document = r#"
[checks]
test = true
coverage = false
size = false
complexity = false
deps = false
mutation = false

[languages]
enabled = ["rust"]

[rust]
roots = ["./", "apps\\service//", "apps/service"]
"#;
        let mut policy: AyniPolicy = toml::from_str(document).expect("parse");
        policy.normalize_and_validate().expect("valid");
        assert_eq!(policy.rust.roots, vec![".", "apps/service"]);
    }

    #[test]
    fn rejects_parent_traversal_root() {
        let document = r#"
[checks]
test = true
coverage = false
size = false
complexity = false
deps = false
mutation = false

[languages]
enabled = ["rust"]

[rust]
roots = ["../outside"]
"#;
        let mut policy: AyniPolicy = toml::from_str(document).expect("parse");
        let error = policy.normalize_and_validate().expect_err("must fail");
        assert!(error.contains("must stay within repository root"));
    }

    #[test]
    fn concurrency_policy_parses() {
        let document = r#"
[checks]
test = true
coverage = true
size = true
complexity = true
deps = true
mutation = false

[languages]
enabled = ["rust"]

[concurrency]
per_language = true
amount = 3
"#;
        let mut policy: AyniPolicy = toml::from_str(document).expect("parse");
        policy.normalize_and_validate().expect("valid");
        assert!(policy.concurrency.per_language);
        assert_eq!(policy.concurrency.amount, 3);
    }

    #[test]
    fn rejects_zero_concurrency_amount() {
        let document = r#"
[checks]
test = true
coverage = true
size = true
complexity = true
deps = true
mutation = false

[languages]
enabled = ["rust"]

[concurrency]
amount = 0
"#;
        let mut policy: AyniPolicy = toml::from_str(document).expect("parse");
        let error = policy.normalize_and_validate().expect_err("must fail");
        assert!(error.contains("at least 1"));
    }
}
