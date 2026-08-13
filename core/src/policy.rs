use crate::adapter::PolicyEffectivenessFacts;
use crate::language::Language;
use crate::signal::SignalKind;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::str::FromStr;

pub const AYNI_POLICY_FILE: &str = ".ayni.toml";

/// An advisory diagnostic for policy that parses successfully but cannot
/// affect a requested signal. Codes are stable for renderer and JSON clients.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PolicyEffectivenessWarning {
    pub code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Language>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signal: Option<SignalKind>,
    pub policy_path: String,
    pub message: String,
}

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

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct FoundationPolicy {
    pub runner: Option<String>,
    pub validate_install: Option<bool>,
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
    pub foundation: Option<FoundationPolicy>,
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
    pub execution: ExecutionPolicy,
    #[serde(default)]
    pub rust: LanguageTooling,
    #[serde(default)]
    pub go: LanguageTooling,
    #[serde(default)]
    pub node: LanguageTooling,
    #[serde(default)]
    pub python: LanguageTooling,
    #[serde(default)]
    pub kotlin: LanguageTooling,
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
pub struct ExecutionPolicy {
    /// Maximum seconds a single adapter tool invocation may run before it is
    /// killed and reported as a timeout failure.
    pub tool_timeout_seconds: u64,
}

impl Default for ExecutionPolicy {
    fn default() -> Self {
        Self {
            tool_timeout_seconds: 1800,
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
        Self::parse(&content)
            .map_err(|error| format!("failed to parse {}: {error}", config_path.display()))
    }

    /// Parse and normalize one policy snapshot already read by the caller.
    /// This lets consumers hash and interpret the exact same bytes.
    pub fn parse(content: &str) -> Result<Self, String> {
        let mut policy = toml::from_str::<Self>(content).map_err(|error| error.to_string())?;
        policy.normalize_and_validate()?;
        Ok(policy)
    }

    #[must_use]
    pub fn enabled_signals(&self) -> Vec<SignalKind> {
        [
            (self.checks.test, SignalKind::Test),
            (self.checks.coverage, SignalKind::Coverage),
            (self.checks.size, SignalKind::Size),
            (self.checks.complexity, SignalKind::Complexity),
            (self.checks.deps, SignalKind::Deps),
            (self.checks.mutation, SignalKind::Mutation),
        ]
        .into_iter()
        .filter_map(|(enabled, signal)| enabled.then_some(signal))
        .collect()
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
                        "languages.enabled contains unsupported language '{value}'; expected rust, go, node, python, or kotlin"
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
            Language::Python => &self.python,
            Language::Kotlin => &self.kotlin,
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

    /// Produce deterministic advisory diagnostics using only policy and static
    /// adapter declarations. This method performs no discovery, tool
    /// execution, filesystem writes, or validation changes.
    #[must_use]
    pub fn effectiveness_warnings(
        &self,
        adapter_facts: &[PolicyEffectivenessFacts],
    ) -> Vec<PolicyEffectivenessWarning> {
        let mut warnings = Vec::new();
        let enabled_languages = self.enabled_languages().unwrap_or_default();
        let mut facts = adapter_facts.to_vec();
        facts.sort_by_key(|fact| fact.language);
        facts.dedup_by_key(|fact| fact.language);

        for language in enabled_languages {
            let tooling = self.language_tooling(language);
            self.disabled_check_configuration_warnings(language, tooling, &mut warnings);
            self.empty_enabled_rule_warnings(language, tooling, &mut warnings);
            self.missing_complexity_threshold_warnings(language, tooling, &facts, &mut warnings);
        }
        warnings.sort_by(|left, right| {
            (&left.language, &left.signal, &left.code, &left.policy_path).cmp(&(
                &right.language,
                &right.signal,
                &right.code,
                &right.policy_path,
            ))
        });
        warnings
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
                    "languages.enabled contains unsupported language '{value}'; expected rust, go, node, python, or kotlin"
                )
            })?;
        }
        self.rust.roots = normalize_roots("rust", &self.rust.roots)?;
        self.go.roots = normalize_roots("go", &self.go.roots)?;
        self.node.roots = normalize_roots("node", &self.node.roots)?;
        self.python.roots = normalize_roots("python", &self.python.roots)?;
        self.kotlin.roots = normalize_roots("kotlin", &self.kotlin.roots)?;
        if self.concurrency.amount == 0 {
            return Err(String::from("concurrency.amount must be at least 1"));
        }
        if self.execution.tool_timeout_seconds == 0 {
            return Err(String::from(
                "execution.tool_timeout_seconds must be at least 1",
            ));
        }
        for (language, tooling) in [
            ("rust", &self.rust),
            ("go", &self.go),
            ("node", &self.node),
            ("python", &self.python),
            ("kotlin", &self.kotlin),
        ] {
            validate_language_thresholds(language, tooling)?;
        }
        Ok(())
    }

    fn disabled_check_configuration_warnings(
        &self,
        language: Language,
        tooling: &LanguageTooling,
        warnings: &mut Vec<PolicyEffectivenessWarning>,
    ) {
        for (kind, configured) in configured_signals(tooling) {
            if configured && !self.signal_enabled(kind) {
                let name = signal_policy_name(kind);
                warnings.push(warning(
                    "policy.effectiveness.disabled_check_hides_configuration",
                    language,
                    kind,
                    format!("checks.{name}"),
                    format!(
                        "{name} policy is configured for {language}, but checks.{name} is disabled"
                    ),
                ));
            }
        }
    }

    fn empty_enabled_rule_warnings(
        &self,
        language: Language,
        tooling: &LanguageTooling,
        warnings: &mut Vec<PolicyEffectivenessWarning>,
    ) {
        if self.signal_enabled(SignalKind::Size) && tooling.size.is_empty() {
            warnings.push(warning(
                "policy.effectiveness.size.no_rules",
                language,
                SignalKind::Size,
                format!("{}.size", language.as_str()),
                format!("size is enabled for {language}, but no size rules are configured"),
            ));
        }
        if self.signal_enabled(SignalKind::Coverage) && coverage_has_no_threshold(tooling) {
            warnings.push(warning(
                "policy.effectiveness.coverage.no_threshold",
                language,
                SignalKind::Coverage,
                format!("{}.coverage", language.as_str()),
                format!(
                    "coverage is enabled for {language}, but no coverage threshold is configured"
                ),
            ));
        }
        if self.signal_enabled(SignalKind::Deps) && deps_has_no_forbidden_edges(tooling) {
            warnings.push(warning(
                "policy.effectiveness.deps.no_forbidden_edges",
                language,
                SignalKind::Deps,
                format!("{}.deps.forbidden", language.as_str()),
                format!("deps is enabled for {language}, but no forbidden dependency edges are configured"),
            ));
        }
    }

    fn missing_complexity_threshold_warnings(
        &self,
        language: Language,
        tooling: &LanguageTooling,
        facts: &[PolicyEffectivenessFacts],
        warnings: &mut Vec<PolicyEffectivenessWarning>,
    ) {
        if !self.signal_enabled(SignalKind::Complexity) {
            return;
        }
        let Some(fact) = facts.iter().find(|fact| fact.language == language) else {
            return;
        };
        for required in &fact.required_complexity_thresholds {
            if !complexity_threshold_configured(tooling, *required) {
                let key = required.policy_key();
                warnings.push(warning(
                    "policy.effectiveness.complexity.missing_required_threshold",
                    language,
                    SignalKind::Complexity,
                    format!("{}.complexity.{key}", language.as_str()),
                    format!("complexity is enabled for {language}, but required threshold {key} is not configured"),
                ));
            }
        }
    }
}

fn warning(
    code: &str,
    language: Language,
    signal: SignalKind,
    policy_path: String,
    message: String,
) -> PolicyEffectivenessWarning {
    PolicyEffectivenessWarning {
        code: String::from(code),
        language: Some(language),
        signal: Some(signal),
        policy_path,
        message,
    }
}

fn signal_policy_name(kind: SignalKind) -> &'static str {
    match kind {
        SignalKind::Test => "test",
        SignalKind::Coverage => "coverage",
        SignalKind::Size => "size",
        SignalKind::Complexity => "complexity",
        SignalKind::Deps => "deps",
        SignalKind::Mutation => "mutation",
    }
}

fn configured_signals(tooling: &LanguageTooling) -> [(SignalKind, bool); 6] {
    [
        (SignalKind::Test, tooling.tooling.test.is_some()),
        (
            SignalKind::Coverage,
            tooling.coverage.is_some() || tooling.tooling.coverage.is_some(),
        ),
        (SignalKind::Size, !tooling.size.is_empty()),
        (SignalKind::Complexity, tooling.complexity.is_some()),
        (SignalKind::Deps, tooling.deps.is_some()),
        (SignalKind::Mutation, tooling.tooling.mutation.is_some()),
    ]
}

fn coverage_has_no_threshold(tooling: &LanguageTooling) -> bool {
    tooling
        .coverage
        .as_ref()
        .is_none_or(|coverage| coverage.line_percent.is_none() && coverage.branch_percent.is_none())
}

fn deps_has_no_forbidden_edges(tooling: &LanguageTooling) -> bool {
    tooling
        .deps
        .as_ref()
        .is_none_or(|deps| deps.forbidden.is_empty())
}

fn complexity_threshold_configured(
    tooling: &LanguageTooling,
    required: crate::adapter::ComplexityThresholdKind,
) -> bool {
    tooling
        .complexity
        .as_ref()
        .is_some_and(|complexity| match required {
            crate::adapter::ComplexityThresholdKind::FnCyclomatic => {
                complexity.fn_cyclomatic.is_some()
            }
            crate::adapter::ComplexityThresholdKind::FnCognitive => {
                complexity.fn_cognitive.is_some()
            }
        })
}

impl AyniPolicy {
    fn signal_enabled(&self, kind: SignalKind) -> bool {
        match kind {
            SignalKind::Test => self.checks.test,
            SignalKind::Coverage => self.checks.coverage,
            SignalKind::Size => self.checks.size,
            SignalKind::Complexity => self.checks.complexity,
            SignalKind::Deps => self.checks.deps,
            SignalKind::Mutation => self.checks.mutation,
        }
    }
}

fn validate_language_thresholds(language: &str, tooling: &LanguageTooling) -> Result<(), String> {
    for (pattern, rule) in &tooling.size {
        if rule.warn > rule.fail {
            return Err(format!(
                "{language}.size rule '{pattern}': warn ({}) must not exceed fail ({})",
                rule.warn, rule.fail
            ));
        }
    }
    if let Some(complexity) = &tooling.complexity {
        for (name, threshold) in [
            ("fn_cyclomatic", complexity.fn_cyclomatic),
            ("fn_cognitive", complexity.fn_cognitive),
        ] {
            if let Some(threshold) = threshold
                && threshold.warn > threshold.fail
            {
                return Err(format!(
                    "{language}.complexity.{name}: warn ({}) must not exceed fail ({})",
                    threshold.warn, threshold.fail
                ));
            }
        }
    }
    if let Some(coverage) = &tooling.coverage {
        for (name, threshold) in [
            ("line_percent", coverage.line_percent),
            ("branch_percent", coverage.branch_percent),
        ] {
            if let Some(threshold) = threshold
                && threshold.warn < threshold.fail
            {
                return Err(format!(
                    "{language}.coverage.{name}: warn ({}) must be at least fail ({}) because coverage thresholds are minimums",
                    threshold.warn, threshold.fail
                ));
            }
        }
    }
    Ok(())
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
    let portable = value.trim().replace('\\', "/");
    let path = Path::new(&portable);
    let has_windows_prefix = portable.as_bytes().get(1) == Some(&b':')
        && portable
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphabetic);
    if path.is_absolute() || portable.starts_with('/') || has_windows_prefix {
        return Err(format!(
            "{language}.roots entry '{value}' must be repo-relative, not absolute"
        ));
    }

    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(component) => normalized.push(component),
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(format!(
                    "{language}.roots entry '{value}' must stay within repository root and cannot contain parent components"
                ));
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(format!(
                    "{language}.roots entry '{value}' must be repo-relative, not absolute"
                ));
            }
        }
    }
    if normalized.as_os_str().is_empty() {
        Ok(String::from("."))
    } else {
        Ok(normalized.to_string_lossy().replace('\\', "/"))
    }
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
    use crate::adapter::{ComplexityThresholdKind, PolicyEffectivenessFacts};
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
    fn python_foundation_policy_parses() {
        let document = r#"
[checks]
test = true
coverage = true
size = true
complexity = true
deps = true
mutation = false

[languages]
enabled = ["python"]

[python.foundation]
runner = "workspace"
validate_install = true
"#;
        let policy: AyniPolicy = toml::from_str(document).expect("parse");
        let foundation = policy
            .python
            .foundation
            .as_ref()
            .expect("python foundation");
        assert_eq!(foundation.runner.as_deref(), Some("workspace"));
        assert_eq!(foundation.validate_install, Some(true));
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
        assert_eq!(policy.roots_for(Language::Python), ["."]);
    }

    #[test]
    fn python_policy_sections_parse() {
        let document = r#"
[checks]
test = true
coverage = true
size = true
complexity = true
deps = true
mutation = false

[languages]
enabled = ["python"]

[python]
roots = ["src"]

[python.size]
"**/*.py" = { warn = 400, fail = 800, exclude = [".venv/**"] }

[python.complexity]
fn_cognitive = { warn = 10, fail = 15 }

[python.coverage]
line_percent = { warn = 80, fail = 60 }

[python.deps.forbidden]
"src/domain/**" = ["src/presentation/**"]
"#;
        let mut policy: AyniPolicy = toml::from_str(document).expect("parse");
        policy.normalize_and_validate().expect("valid");
        assert_eq!(
            policy.enabled_languages().expect("languages"),
            [Language::Python]
        );
        assert_eq!(policy.roots_for(Language::Python), ["src"]);
        assert_eq!(
            policy
                .size_rules_for(Language::Python)
                .get("**/*.py")
                .expect("size")
                .fail,
            800
        );
        assert_eq!(
            policy
                .python
                .complexity
                .as_ref()
                .expect("complexity")
                .fn_cognitive
                .expect("cognitive")
                .fail,
            15.0
        );
        assert_eq!(
            policy
                .python
                .coverage
                .as_ref()
                .expect("coverage")
                .line_percent
                .expect("coverage threshold")
                .warn,
            80.0
        );
        assert_eq!(
            policy
                .python
                .deps
                .as_ref()
                .expect("deps")
                .forbidden
                .get("src/domain/**")
                .expect("rule"),
            &vec![String::from("src/presentation/**")]
        );
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
    fn rejects_parent_components() {
        for root in [
            "..",
            "./..",
            "../outside",
            "apps/../outside",
            "apps/./../outside",
        ] {
            let error = normalize_root_entry("rust", root).expect_err("must fail");
            assert!(
                error.contains("must stay within repository root"),
                "{root}: {error}"
            );
        }
    }

    #[test]
    fn rejects_absolute_rooted_and_windows_prefixed_roots() {
        for root in [
            "/outside",
            "\\outside",
            "C:/outside",
            "c:\\outside",
            "D:outside",
            "\\\\server\\share",
        ] {
            let error = normalize_root_entry("rust", root).expect_err("must fail");
            assert!(error.contains("repo-relative"), "{root}: {error}");
        }
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

    #[test]
    fn effectiveness_warnings_cover_empty_enabled_rules_and_required_thresholds() {
        let document = r#"
[checks]
test = false
coverage = true
size = true
complexity = true
deps = true
mutation = false

[languages]
enabled = ["rust", "python"]

[rust.complexity]
fn_cognitive = { warn = 10, fail = 20 }

[python.coverage]
branch_percent = { warn = 80, fail = 70 }
"#;
        let policy: AyniPolicy = toml::from_str(document).expect("parse");
        let warnings = policy.effectiveness_warnings(&[
            PolicyEffectivenessFacts::new(
                Language::Rust,
                vec![ComplexityThresholdKind::FnCyclomatic],
            ),
            PolicyEffectivenessFacts::new(
                Language::Python,
                vec![ComplexityThresholdKind::FnCognitive],
            ),
        ]);

        let actual = warnings
            .iter()
            .map(|warning| (warning.code.as_str(), warning.policy_path.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(
            actual,
            vec![
                (
                    "policy.effectiveness.coverage.no_threshold",
                    "rust.coverage"
                ),
                ("policy.effectiveness.size.no_rules", "rust.size"),
                (
                    "policy.effectiveness.complexity.missing_required_threshold",
                    "rust.complexity.fn_cyclomatic"
                ),
                (
                    "policy.effectiveness.deps.no_forbidden_edges",
                    "rust.deps.forbidden"
                ),
                ("policy.effectiveness.size.no_rules", "python.size"),
                (
                    "policy.effectiveness.complexity.missing_required_threshold",
                    "python.complexity.fn_cognitive"
                ),
                (
                    "policy.effectiveness.deps.no_forbidden_edges",
                    "python.deps.forbidden"
                ),
            ]
        );
        assert!(warnings.iter().all(|warning| warning.language.is_some()));
        assert!(warnings.iter().all(|warning| warning.signal.is_some()));
    }

    #[test]
    fn effectiveness_warnings_report_configuration_hidden_by_disabled_check() {
        let document = r#"
[checks]
test = false
coverage = false
size = false
complexity = false
deps = false
mutation = false

[languages]
enabled = ["rust"]

[rust.size]
"*.rs" = { warn = 400, fail = 700 }

[rust.coverage]
line_percent = { warn = 80, fail = 70 }

[rust.complexity]
fn_cyclomatic = { warn = 10, fail = 20 }

[rust.deps.forbidden]
"src" = ["legacy"]

[rust.tooling.test]
command = "cargo"

[rust.tooling.mutation]
command = "cargo"
"#;
        let policy: AyniPolicy = toml::from_str(document).expect("parse");
        let warnings = policy.effectiveness_warnings(&[]);
        assert_eq!(warnings.len(), 6);
        assert!(warnings.iter().all(|warning| {
            warning.code == "policy.effectiveness.disabled_check_hides_configuration"
        }));
        assert_eq!(warnings[0].policy_path, "checks.test");
        assert_eq!(warnings[5].policy_path, "checks.mutation");
    }
}
