use crate::catalog::{CatalogEntry, CatalogRuntime};
use crate::language::Language;
use crate::runtime::Scope;
use crate::runtime::{AdapterError, ExecutionResolution, RunContext};
use crate::signal::{
    FindingError, Findings, OffenderIdentity, SignalKind, SignalRow, VerificationTarget,
};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Complexity thresholds that an adapter needs in order to measure its
/// complexity signal. This is a declarative capability: reading it must not
/// discover a project or invoke a tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ComplexityThresholdKind {
    FnCyclomatic,
    FnCognitive,
}

impl ComplexityThresholdKind {
    #[must_use]
    pub const fn policy_key(self) -> &'static str {
        match self {
            Self::FnCyclomatic => "fn_cyclomatic",
            Self::FnCognitive => "fn_cognitive",
        }
    }
}

/// Adapter-owned facts used to diagnose valid configuration that cannot
/// produce an effective measurement. These facts deliberately contain no
/// repository state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyEffectivenessFacts {
    pub language: Language,
    pub required_complexity_thresholds: Vec<ComplexityThresholdKind>,
}

impl PolicyEffectivenessFacts {
    #[must_use]
    pub fn new(
        language: Language,
        mut required_complexity_thresholds: Vec<ComplexityThresholdKind>,
    ) -> Self {
        required_complexity_thresholds.sort();
        required_complexity_thresholds.dedup();
        Self {
            language,
            required_complexity_thresholds,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct DetectResult {
    pub detected: bool,
    pub confidence: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LanguageProfile {
    pub language: Language,
    pub default_file_globs: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectLayout {
    SingleRoot,
    ControlledMonorepo,
    UncontrolledMonorepo,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveredRoot {
    pub path: String,
    pub analyzable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectDiscovery {
    pub layout: ProjectLayout,
    pub roots: Vec<DiscoveredRoot>,
}

impl ProjectDiscovery {
    #[must_use]
    pub fn from_analyzable_roots(mut roots: Vec<String>) -> Self {
        roots.sort();
        roots.dedup();
        let layout = match roots.as_slice() {
            [root] if root == "." => ProjectLayout::SingleRoot,
            [_] => ProjectLayout::UncontrolledMonorepo,
            _ => ProjectLayout::UncontrolledMonorepo,
        };
        Self {
            layout,
            roots: roots
                .into_iter()
                .map(|path| DiscoveredRoot {
                    path,
                    analyzable: true,
                })
                .collect(),
        }
    }

    #[must_use]
    pub fn policy_roots(&self) -> Vec<String> {
        let roots = self.analyzable_roots();
        if roots.is_empty() {
            vec![String::from(".")]
        } else {
            roots
        }
    }

    #[must_use]
    pub fn analyzable_roots(&self) -> Vec<String> {
        let mut roots: Vec<String> = self
            .roots
            .iter()
            .filter(|root| root.analyzable)
            .map(|root| root.path.clone())
            .collect();
        roots.sort();
        roots.dedup();
        roots
    }
}

pub trait SignalCollector: Send + Sync {
    fn collect(&self, kind: SignalKind, context: &RunContext) -> Result<SignalRow, AdapterError>;

    /// Collect while streaming live tool output lines through `on_line`.
    /// The default implementation ignores streaming and delegates to
    /// [`Self::collect`]; adapters whose tools produce useful progress output
    /// (long test runs, for example) should override it.
    fn collect_streaming(
        &self,
        kind: SignalKind,
        context: &RunContext,
        on_line: &mut dyn FnMut(&str),
    ) -> Result<SignalRow, AdapterError> {
        let _ = on_line;
        self.collect(kind, context)
    }

    /// Collect one signal using adapter-owned focused selector behavior.
    ///
    /// Callers should enter through [`LanguageAdapter::collect_verification`],
    /// which rejects unsupported selectors before this method can invoke a
    /// tool.
    fn collect_verification(
        &self,
        kind: SignalKind,
        context: &RunContext,
        selection: &VerificationSelection,
        on_line: &mut dyn FnMut(&str),
    ) -> Result<SignalRow, AdapterError> {
        let _ = selection;
        self.collect_streaming(kind, context, on_line)
    }
}

/// Signal-neutral selectors for a requested verification run.
///
/// Language selection is deliberately absent: choosing an adapter is an
/// orchestration concern, not a capability of an adapter's underlying tools.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VerificationSelection {
    pub file: Option<String>,
    pub package: Option<String>,
    pub name: Option<String>,
}

impl VerificationSelection {
    #[must_use]
    pub fn is_unscoped(&self) -> bool {
        self.file.is_none() && self.package.is_none() && self.name.is_none()
    }
}

/// Selectors an adapter truthfully applies while measuring one signal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct VerificationSelectorSupport {
    pub file: bool,
    pub package: bool,
    pub name: bool,
}

impl VerificationSelectorSupport {
    pub const NONE: Self = Self::new(false, false, false);

    #[must_use]
    pub const fn new(file: bool, package: bool, name: bool) -> Self {
        Self {
            file,
            package,
            name,
        }
    }

    fn unsupported(self, selection: &VerificationSelection) -> Vec<&'static str> {
        let mut unsupported = Vec::new();
        if selection.file.is_some() && !self.file {
            unsupported.push("file");
        }
        if selection.package.is_some() && !self.package {
            unsupported.push("package");
        }
        if selection.name.is_some() && !self.name {
            unsupported.push("name");
        }
        unsupported
    }

    fn supported_names(self) -> Vec<&'static str> {
        let mut supported = Vec::new();
        if self.file {
            supported.push("file");
        }
        if self.package {
            supported.push("package");
        }
        if self.name {
            supported.push("name");
        }
        supported
    }

    /// Ensure an adapter-produced finding target does not claim selectors that
    /// its focused collector cannot apply.
    pub fn validate_target(
        self,
        kind: SignalKind,
        target: &VerificationTarget,
    ) -> Result<(), FindingError> {
        target.validate()?;
        if target.file.is_some() && !self.file {
            return Err(FindingError::UnsupportedVerificationSelector("file"));
        }
        if target.package.is_some() && !self.package {
            return Err(FindingError::UnsupportedVerificationSelector("package"));
        }
        if target.name.is_some() && (kind != SignalKind::Test || !self.name) {
            return Err(FindingError::UnsupportedVerificationSelector("name"));
        }
        Ok(())
    }
}

pub trait LanguageAdapter: Send + Sync {
    fn language(&self) -> Language;
    fn detect(&self, root: &Path) -> DetectResult;
    fn resolve_execution(&self, _repo_root: &Path, root: &Path) -> Option<ExecutionResolution> {
        self.detect(root).detected.then(|| {
            ExecutionResolution::direct(
                self.language().as_str(),
                root.to_path_buf(),
                format!("{} root", self.language().as_str()),
                60,
            )
        })
    }
    fn discover_roots(&self, repo_root: &Path) -> Vec<String>;
    fn discover_project_roots(&self, repo_root: &Path) -> ProjectDiscovery {
        ProjectDiscovery::from_analyzable_roots(self.discover_roots(repo_root))
    }
    fn profile(&self) -> LanguageProfile;
    fn catalog(&self) -> &'static [CatalogEntry];
    /// Runtime behavior for the ordered declarative catalog.
    fn catalog_runtime(&self) -> &dyn CatalogRuntime;
    fn collector(&self) -> &dyn SignalCollector;

    /// Optional environment-planning capability. Existing quality adapters
    /// remain valid while environment discovery is implemented incrementally.
    fn environment_capability(&self) -> Option<&dyn crate::EnvironmentCapability> {
        None
    }

    /// Optional native dependency preparation capability. The returned commands
    /// are semantic, structured argv and are intended for an isolated staged
    /// workspace; adapters never execute them through this wrapper.
    fn dependency_preparation_capability(
        &self,
    ) -> Option<&dyn crate::DependencyPreparationCapability> {
        None
    }

    fn prepare_dependencies(
        &self,
        request: &crate::DependencyPreparationRequest,
    ) -> Result<crate::DependencyPreparationPlan, AdapterError> {
        let capability = self.dependency_preparation_capability().ok_or_else(|| {
            AdapterError::new(
                self.language(),
                "dependency preparation capability is unsupported",
            )
        })?;
        if request.target().target.language != self.language()
            || capability.language() != self.language()
        {
            return Err(AdapterError::new(
                self.language(),
                "dependency preparation language does not match adapter language",
            ));
        }
        let plan = capability.prepare(request)?;
        if plan.target != request.target().target {
            return Err(AdapterError::new(
                self.language(),
                "dependency preparation target does not match the requested target",
            ));
        }
        Ok(plan)
    }

    /// Optional exact-resolution capability used only by explicit environment locking.
    fn environment_resolution_capability(
        &self,
    ) -> Option<&dyn crate::EnvironmentResolutionCapability> {
        None
    }

    fn resolve_environment(
        &self,
        request: &crate::EnvironmentResolutionRequest,
    ) -> Result<crate::TargetEnvironment, AdapterError> {
        let capability = self.environment_resolution_capability().ok_or_else(|| {
            AdapterError::new(
                self.language(),
                "environment resolution capability is unsupported",
            )
        })?;
        if request.target().target.language != self.language()
            || capability.language() != self.language()
        {
            return Err(AdapterError::new(
                self.language(),
                "environment resolution language does not match adapter language",
            ));
        }
        let resolved = capability.resolve(request)?;
        if resolved.target != request.target().target {
            return Err(AdapterError::new(
                self.language(),
                "resolved environment target does not match the requested target",
            ));
        }
        Ok(resolved)
    }

    /// Validate capability language and target identity around one read-only
    /// environment discovery call.
    fn discover_environment(
        &self,
        request: &crate::EnvironmentDiscoveryRequest,
    ) -> Result<crate::EnvironmentContribution, AdapterError> {
        let capability = self.environment_capability().ok_or_else(|| {
            AdapterError::new(
                self.language(),
                "environment discovery capability is unsupported",
            )
        })?;
        if request.target().language != self.language() {
            return Err(AdapterError::new(
                self.language(),
                "environment request language does not match adapter language",
            ));
        }
        if capability.language() != self.language() {
            return Err(AdapterError::new(
                self.language(),
                "environment capability reports a different language",
            ));
        }
        let contribution = capability.discover(request)?;
        crate::environment_adapter::validate_environment_contribution(
            self.language(),
            request,
            &contribution,
        )?;
        Ok(contribution)
    }

    /// Return static policy requirements for this adapter. Implementations
    /// must not inspect the filesystem, discover roots, or execute tools.
    fn policy_effectiveness_facts(&self) -> PolicyEffectivenessFacts {
        PolicyEffectivenessFacts::new(self.language(), Vec::new())
    }

    /// Declare selectors that produce genuinely scoped measurements for a
    /// canonical signal. The default is intentionally fail-closed.
    fn verification_selector_support(&self, _kind: SignalKind) -> VerificationSelectorSupport {
        VerificationSelectorSupport::NONE
    }

    /// Map one typed offender to selectors understood by this adapter.
    /// Implementations return data only; CLI command rendering is deliberately
    /// outside the adapter boundary.
    fn verification_target(
        &self,
        _kind: SignalKind,
        _scope: &Scope,
        _offender: OffenderIdentity<'_>,
    ) -> VerificationTarget {
        VerificationTarget::default()
    }

    /// Convert a collected row's offenders into stable, deduplicated findings
    /// and fail closed if target mapping disagrees with declared capability.
    fn findings_for(&self, row: &SignalRow, checkout_root: &str) -> Result<Findings, FindingError> {
        let support = self.verification_selector_support(row.kind);
        let findings =
            row.offenders
                .clone()
                .into_findings(row.language, checkout_root, |offender| {
                    self.verification_target(row.kind, &row.scope, offender)
                })?;
        findings.validate_targets(row.kind, support)?;
        Ok(findings)
    }

    /// Reject selectors that this adapter cannot apply to the requested
    /// signal. This method performs no discovery or tool execution.
    fn validate_verification_selection(
        &self,
        kind: SignalKind,
        selection: &VerificationSelection,
    ) -> Result<(), AdapterError> {
        validate_selector_support(
            self.language(),
            kind,
            self.verification_selector_support(kind),
            selection,
        )
    }

    /// Validate selector support and collect only after validation succeeds.
    fn collect_verification(
        &self,
        kind: SignalKind,
        context: &RunContext,
        selection: &VerificationSelection,
        on_line: &mut dyn FnMut(&str),
    ) -> Result<SignalRow, AdapterError> {
        self.validate_verification_selection(kind, selection)?;
        if selection.file != context.scope.file || selection.package != context.scope.package {
            return Err(AdapterError::new(
                self.language(),
                "verification selection does not match the run-context scope",
            ));
        }
        self.collector()
            .collect_verification(kind, context, selection, on_line)
    }

    /// Maximum number of analyze targets for this language that may run
    /// concurrently. `None` means the global concurrency policy applies
    /// unchanged. Adapters whose tooling serializes on shared state (for
    /// example Cargo's target-directory lock) should return `Some(1)`.
    fn max_target_concurrency(&self) -> Option<usize> {
        None
    }
}

fn validate_selector_support(
    language: Language,
    kind: SignalKind,
    support: VerificationSelectorSupport,
    selection: &VerificationSelection,
) -> Result<(), AdapterError> {
    if selection.file.is_some() && selection.package.is_some() {
        return Err(AdapterError::new(
            language,
            format!(
                "{} verification cannot combine file and package selectors",
                signal_name(kind)
            ),
        ));
    }
    if kind != SignalKind::Test && selection.name.is_some() {
        return Err(AdapterError::new(
            language,
            format!(
                "{} verification does not support the test-only name selector",
                signal_name(kind)
            ),
        ));
    }
    let unsupported = support.unsupported(selection);
    if unsupported.is_empty() {
        return Ok(());
    }
    let supported = support.supported_names();
    let alternatives = if supported.is_empty() {
        String::from("no focused selectors")
    } else {
        supported.join(", ")
    };
    Err(AdapterError::new(
        language,
        format!(
            "{} verification does not support selector(s): {}; supported: {alternatives}",
            signal_name(kind),
            unsupported.join(", ")
        ),
    ))
}

fn signal_name(kind: SignalKind) -> &'static str {
    match kind {
        SignalKind::Test => "test",
        SignalKind::Coverage => "coverage",
        SignalKind::Size => "size",
        SignalKind::Complexity => "complexity",
        SignalKind::Deps => "deps",
        SignalKind::Mutation => "mutation",
    }
}

#[cfg(test)]
mod tests {
    use super::{VerificationSelection, VerificationSelectorSupport, validate_selector_support};
    use crate::{
        AdapterError, Architecture, CatalogEntry, CatalogRuntime, DetectResult,
        EnvironmentCapability, EnvironmentContribution, EnvironmentDiscoveryRequest,
        ExecutionResolution, Language, LanguageAdapter, LanguageProfile, Libc, OperatingSystem,
        RequirementConfidence, RequirementSource, RunContext, RuntimeRequirement, SignalCollector,
        SignalKind, SignalRow, TargetEnvironment, TargetIdentity, TargetPlatform,
        VerificationTarget, VersionRequirement,
    };
    use std::path::Path;
    use std::time::Duration;

    struct TestAdapter<'a> {
        capability: Option<&'a dyn EnvironmentCapability>,
    }

    struct TestCapability {
        language: Language,
        target: TargetIdentity,
    }

    struct TestCollector;
    struct TestCatalogRuntime;

    static TEST_COLLECTOR: TestCollector = TestCollector;
    static TEST_CATALOG_RUNTIME: TestCatalogRuntime = TestCatalogRuntime;

    impl EnvironmentCapability for TestCapability {
        fn language(&self) -> Language {
            self.language
        }

        fn discover(
            &self,
            _request: &EnvironmentDiscoveryRequest,
        ) -> Result<EnvironmentContribution, AdapterError> {
            EnvironmentContribution::new(
                TargetEnvironment {
                    target: self.target.clone(),
                    workspace: None,
                    package: None,
                    runtimes: vec![RuntimeRequirement {
                        runtime: String::from("rust"),
                        version: VersionRequirement::exact("1.93.0").expect("version"),
                        components: Vec::new(),
                        targets: Vec::new(),
                        source: RequirementSource::new(
                            "manifest",
                            "Cargo.toml",
                            None::<String>,
                            RequirementConfidence::Declared,
                        )
                        .expect("source"),
                    }],
                    package_manager: None,
                    signal_tools: Vec::new(),
                    system_requirements: Vec::new(),
                    dependency_locks: Vec::new(),
                },
                Vec::new(),
                Vec::new(),
            )
            .map_err(|error| AdapterError::new(self.language, error.to_string()))
        }
    }

    impl SignalCollector for TestCollector {
        fn collect(
            &self,
            _kind: SignalKind,
            _context: &RunContext,
        ) -> Result<SignalRow, AdapterError> {
            panic!("quality collection is outside this test")
        }
    }

    impl CatalogRuntime for TestCatalogRuntime {
        fn status(
            &self,
            _entry: &CatalogEntry,
            _execution: &ExecutionResolution,
            _timeout: Duration,
        ) -> Result<crate::ToolStatus, crate::CatalogOperationError> {
            panic!("catalog execution is outside this test")
        }

        fn install(
            &self,
            _entry: &CatalogEntry,
            _execution: &ExecutionResolution,
            _timeout: Duration,
            _on_line: &mut dyn FnMut(&str),
        ) -> Result<(), crate::CatalogOperationError> {
            panic!("catalog execution is outside this test")
        }
    }

    impl LanguageAdapter for TestAdapter<'_> {
        fn language(&self) -> Language {
            Language::Rust
        }

        fn detect(&self, _root: &Path) -> DetectResult {
            DetectResult::default()
        }

        fn discover_roots(&self, _repo_root: &Path) -> Vec<String> {
            Vec::new()
        }

        fn profile(&self) -> LanguageProfile {
            LanguageProfile {
                language: Language::Rust,
                default_file_globs: Vec::new(),
            }
        }

        fn catalog(&self) -> &'static [CatalogEntry] {
            &[]
        }

        fn catalog_runtime(&self) -> &dyn CatalogRuntime {
            &TEST_CATALOG_RUNTIME
        }

        fn collector(&self) -> &dyn SignalCollector {
            &TEST_COLLECTOR
        }

        fn environment_capability(&self) -> Option<&dyn EnvironmentCapability> {
            self.capability
        }
    }

    fn environment_request(language: Language) -> EnvironmentDiscoveryRequest {
        EnvironmentDiscoveryRequest::new(
            std::env::current_dir().expect("current directory"),
            TargetIdentity::new(language, ".").expect("target"),
            [],
            vec![TargetPlatform {
                os: OperatingSystem::Linux,
                architecture: Architecture::Amd64,
                libc: Libc::Glibc,
            }],
        )
        .expect("request")
    }

    #[test]
    fn selector_support_rejects_unsupported_selectors_with_alternatives() {
        let error = validate_selector_support(
            Language::Rust,
            SignalKind::Test,
            VerificationSelectorSupport::new(false, true, true),
            &VerificationSelection {
                file: Some(String::from("tests/api.rs")),
                package: None,
                name: None,
            },
        )
        .expect_err("Rust test file selection must be rejected");

        assert_eq!(error.language, Language::Rust);
        assert!(error.message.contains("file"));
        assert!(error.message.contains("supported: package, name"));
    }

    #[test]
    fn file_and_package_cannot_be_combined() {
        let error = validate_selector_support(
            Language::Node,
            SignalKind::Test,
            VerificationSelectorSupport::new(true, true, true),
            &VerificationSelection {
                file: Some(String::from("tests/api.test.ts")),
                package: Some(String::from("api")),
                name: None,
            },
        )
        .expect_err("file and package must be mutually exclusive");

        assert!(error.message.contains("cannot combine file and package"));
    }

    #[test]
    fn name_is_test_only_even_if_an_adapter_overclaims_it() {
        let error = validate_selector_support(
            Language::Node,
            SignalKind::Complexity,
            VerificationSelectorSupport::new(false, false, true),
            &VerificationSelection {
                name: Some(String::from("creates user")),
                ..VerificationSelection::default()
            },
        )
        .expect_err("name must remain test-only");

        assert!(error.message.contains("test-only name selector"));
    }

    #[test]
    fn unscoped_verification_is_always_supported() {
        assert!(
            validate_selector_support(
                Language::Python,
                SignalKind::Mutation,
                VerificationSelectorSupport::NONE,
                &VerificationSelection::default(),
            )
            .is_ok()
        );
    }

    #[test]
    fn language_adapter_defaults_to_unsupported_environment_capability() {
        let adapter = TestAdapter { capability: None };
        let error = adapter
            .discover_environment(&environment_request(Language::Rust))
            .expect_err("unsupported capability must fail explicitly");
        assert!(error.message.contains("unsupported"));
    }

    #[test]
    fn language_adapter_rejects_capability_language_and_target_drift() {
        let language_drift = TestCapability {
            language: Language::Node,
            target: TargetIdentity::new(Language::Rust, ".").expect("target"),
        };
        let adapter = TestAdapter {
            capability: Some(&language_drift),
        };
        assert!(
            adapter
                .discover_environment(&environment_request(Language::Rust))
                .expect_err("language drift")
                .message
                .contains("different language")
        );

        let request_drift = TestCapability {
            language: Language::Rust,
            target: TargetIdentity::new(Language::Rust, ".").expect("target"),
        };
        let adapter = TestAdapter {
            capability: Some(&request_drift),
        };
        assert!(
            adapter
                .discover_environment(&environment_request(Language::Node))
                .expect_err("request language drift")
                .message
                .contains("does not match adapter")
        );

        let target_drift = TestCapability {
            language: Language::Rust,
            target: TargetIdentity::new(Language::Rust, "other").expect("target"),
        };
        let adapter = TestAdapter {
            capability: Some(&target_drift),
        };
        assert!(
            adapter
                .discover_environment(&environment_request(Language::Rust))
                .expect_err("target drift")
                .message
                .contains("does not match")
        );
    }

    #[test]
    fn finding_target_validation_rejects_capability_drift() {
        let error = VerificationSelectorSupport::new(false, true, true)
            .validate_target(
                SignalKind::Test,
                &VerificationTarget {
                    file: Some(String::from("tests/api.rs")),
                    ..VerificationTarget::default()
                },
            )
            .expect_err("mapping must not invent file support");

        assert!(error.to_string().contains("unsupported file selector"));
    }
}
