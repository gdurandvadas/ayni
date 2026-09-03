use crate::adapter::PolicyEffectivenessFacts;
use crate::environment::{
    DockerAccess, EnvironmentCapabilities, EnvironmentResourceLimits, NetworkAccess,
};
use crate::environment_provisioning::normalize_debian_package_spec;
use crate::language::Language;
use crate::signal::SignalKind;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
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
    /// Explicitly attests that the configured coverage command executes the
    /// complete required test suite and emits test evidence the adapter can parse.
    pub coverage_satisfies_test: bool,
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

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct DebianEnvironmentPolicy {
    /// Debian package specifications installed into the repository image.
    /// Entries may be names or exact `name=version` specifications.
    pub packages: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct DockerEnvironmentPolicy {
    pub access: DockerAccess,
    pub network: NetworkAccess,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct EnvironmentPolicy {
    /// Exact repository-wide tools installed by Mise in addition to
    /// adapter-inferred language and quality tooling.
    pub tools: BTreeMap<String, String>,
    pub debian: DebianEnvironmentPolicy,
    pub docker: DockerEnvironmentPolicy,
    pub resources: EnvironmentResourceLimits,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct AyniPolicy {
    pub checks: PolicyChecks,
    pub languages: LanguageSelection,
    pub report: ReportPolicy,
    pub concurrency: ConcurrencyPolicy,
    pub execution: ExecutionPolicy,
    pub environment: EnvironmentPolicy,
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
    pub fn environment_tools(&self) -> &BTreeMap<String, String> {
        &self.environment.tools
    }

    #[must_use]
    pub fn environment_debian_packages(&self) -> &[String] {
        &self.environment.debian.packages
    }

    #[must_use]
    pub const fn environment_capabilities(&self) -> EnvironmentCapabilities {
        EnvironmentCapabilities {
            docker: self.environment.docker.access,
            network: self.environment.docker.network,
        }
    }

    #[must_use]
    pub const fn environment_resource_limits(&self) -> EnvironmentResourceLimits {
        self.environment.resources
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
            self.coverage_reuse_warnings(language, tooling, &mut warnings);
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
        validate_enabled_languages(&self.languages.enabled)?;
        normalize_policy_roots(self)?;
        normalize_environment_policy(&mut self.environment)?;
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

    fn coverage_reuse_warnings(
        &self,
        language: Language,
        tooling: &LanguageTooling,
        warnings: &mut Vec<PolicyEffectivenessWarning>,
    ) {
        if tooling.tooling.coverage_satisfies_test
            && (!self.signal_enabled(SignalKind::Test)
                || !self.signal_enabled(SignalKind::Coverage))
        {
            warnings.push(warning(
                "policy.effectiveness.coverage_reuse.inactive",
                language,
                SignalKind::Coverage,
                format!("{}.tooling.coverage_satisfies_test", language.as_str()),
                String::from(
                    "coverage_satisfies_test requires both checks.test and checks.coverage to be enabled",
                ),
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
            tooling.coverage.is_some()
                || tooling.tooling.coverage.is_some()
                || tooling.tooling.coverage_satisfies_test,
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

fn validate_enabled_languages(enabled: &[String]) -> Result<(), String> {
    if enabled.is_empty() {
        return Err(String::from(
            "languages.enabled must be an explicit non-empty list (for example: [\"rust\"])",
        ));
    }
    let mut seen = std::collections::BTreeSet::new();
    for value in enabled {
        if !seen.insert(value) {
            return Err(format!(
                "languages.enabled contains duplicate language '{value}'"
            ));
        }
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
    Ok(())
}

fn normalize_policy_roots(policy: &mut AyniPolicy) -> Result<(), String> {
    policy.rust.roots = normalize_roots("rust", &policy.rust.roots)?;
    policy.go.roots = normalize_roots("go", &policy.go.roots)?;
    policy.node.roots = normalize_roots("node", &policy.node.roots)?;
    policy.python.roots = normalize_roots("python", &policy.python.roots)?;
    policy.kotlin.roots = normalize_roots("kotlin", &policy.kotlin.roots)?;
    Ok(())
}

fn normalize_environment_policy(environment: &mut EnvironmentPolicy) -> Result<(), String> {
    normalize_environment_tools(&mut environment.tools)?;
    normalize_debian_packages(&mut environment.debian.packages)?;
    environment.resources.validate()
}

fn normalize_environment_tools(tools: &mut BTreeMap<String, String>) -> Result<(), String> {
    let declared = std::mem::take(tools);
    for (raw_tool, raw_version) in declared {
        let tool = raw_tool.trim().to_ascii_lowercase();
        if tool.is_empty()
            || !tool.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':' | b'/')
            })
        {
            return Err(format!(
                "environment.tools key '{raw_tool}' is not a valid Mise tool identifier"
            ));
        }
        let version = raw_version.trim().to_owned();
        if version.is_empty() {
            return Err(format!(
                "environment.tools.{tool} must declare a non-empty exact version"
            ));
        }
        crate::environment::reject_floating_version(&version).map_err(|_| {
            format!("environment.tools.{tool} must declare a non-floating exact version")
        })?;
        if tools.insert(tool.clone(), version).is_some() {
            return Err(format!(
                "environment.tools contains duplicate normalized tool '{tool}'"
            ));
        }
    }
    Ok(())
}

fn normalize_debian_packages(packages: &mut Vec<String>) -> Result<(), String> {
    for package in packages.iter_mut() {
        *package = normalize_debian_package_spec(package.clone()).map_err(|_| {
            format!(
                "environment.debian.packages contains invalid package specification '{package}'"
            )
        })?;
    }
    packages.sort();
    packages.dedup();
    Ok(())
}

fn validate_language_thresholds(language: &str, tooling: &LanguageTooling) -> Result<(), String> {
    validate_tool_command_overrides(language, &tooling.tooling)?;
    validate_size_thresholds(language, &tooling.size)?;
    if let Some(complexity) = &tooling.complexity {
        for (name, threshold) in [
            ("fn_cyclomatic", complexity.fn_cyclomatic),
            ("fn_cognitive", complexity.fn_cognitive),
        ] {
            if let Some(threshold) = threshold {
                validate_nonnegative_finite_threshold(
                    &format!("{language}.complexity.{name}"),
                    threshold,
                )?;
                if threshold.warn > threshold.fail {
                    return Err(format!(
                        "{language}.complexity.{name}: warn ({}) must not exceed fail ({})",
                        threshold.warn, threshold.fail
                    ));
                }
            }
        }
    }
    if let Some(coverage) = &tooling.coverage {
        for (name, threshold) in [
            ("line_percent", coverage.line_percent),
            ("branch_percent", coverage.branch_percent),
        ] {
            if let Some(threshold) = threshold {
                validate_percentage_threshold(&format!("{language}.coverage.{name}"), threshold)?;
                if threshold.warn < threshold.fail {
                    return Err(format!(
                        "{language}.coverage.{name}: warn ({}) must be at least fail ({}) because coverage thresholds are minimums",
                        threshold.warn, threshold.fail
                    ));
                }
            }
        }
    }
    Ok(())
}

fn validate_size_thresholds(
    language: &str,
    size: &BTreeMap<String, SizeThreshold>,
) -> Result<(), String> {
    for (pattern, rule) in size {
        if rule.warn > rule.fail {
            return Err(format!(
                "{language}.size rule '{pattern}': warn ({}) must not exceed fail ({})",
                rule.warn, rule.fail
            ));
        }
    }
    Ok(())
}

fn validate_tool_command_overrides(
    language: &str,
    overrides: &LanguageToolingOverrides,
) -> Result<(), String> {
    for (signal, override_command) in [
        ("test", overrides.test.as_ref()),
        ("coverage", overrides.coverage.as_ref()),
        ("mutation", overrides.mutation.as_ref()),
    ] {
        if override_command
            .is_some_and(|override_command| override_command.command.trim().is_empty())
        {
            return Err(format!(
                "{language}.tooling.{signal}.command must be a non-empty command"
            ));
        }
    }
    Ok(())
}

fn validate_nonnegative_finite_threshold(
    key: &str,
    threshold: ThresholdFloat,
) -> Result<(), String> {
    for (name, value) in [("warn", threshold.warn), ("fail", threshold.fail)] {
        if !value.is_finite() || value < 0.0 {
            return Err(format!("{key}.{name} must be finite and at least 0"));
        }
    }
    Ok(())
}

fn validate_percentage_threshold(key: &str, threshold: ThresholdFloat) -> Result<(), String> {
    for (name, value) in [("warn", threshold.warn), ("fail", threshold.fail)] {
        if !value.is_finite() || !(0.0..=100.0).contains(&value) {
            return Err(format!("{key}.{name} must be finite and between 0 and 100"));
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
#[path = "policy_tests.rs"]
mod tests;
