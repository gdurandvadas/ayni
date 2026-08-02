//! Core contracts for Ayni's unified signal model (foundations / pre-1.0).

pub mod adapter;
pub mod catalog;
pub mod comparison;
pub mod finding;
pub mod language;
pub mod policy;
pub mod registry;
pub mod runtime;
pub mod signal;
pub mod size;
pub mod threshold;

pub use adapter::{
    ComplexityThresholdKind, DetectResult, DiscoveredRoot, LanguageAdapter, LanguageProfile,
    PolicyEffectivenessFacts, ProjectDiscovery, ProjectLayout, SignalCollector,
    VerificationSelection, VerificationSelectorSupport,
};
pub use catalog::{
    CatalogEntry, InstallContext, Installer, NodePackageManager, PythonPackageManager,
    PythonPackageManagerResolution, PythonResolutionKind, ToolStatus, VersionCheck,
    detect_node_package_manager, detect_python_package_manager, resolve_python_package_manager,
};
pub use comparison::{
    ARTIFACT_COMPARISON_SCHEMA_VERSION, ArtifactComparison, ArtifactComparisonError,
    FindingIdChanges, MatchedRowComparison, MetricChange, MetricValue, RowChangeSet, SignalRowKey,
    ValueChange, compare_artifacts,
};
pub use language::Language;
pub use policy::{
    AYNI_POLICY_FILE, AyniPolicy, ComplexityPolicy, ConcurrencyPolicy, CoveragePolicy, DepsPolicy,
    ExecutionPolicy, FoundationPolicy, LanguageSelection, LanguageTooling,
    LanguageToolingOverrides, PolicyChecks, PolicyEffectivenessWarning, ReportPolicy,
    SizeThreshold, ThresholdFloat, ThresholdInt, ToolCommandOverride,
};
pub use registry::AdapterRegistry;
pub use runtime::{AdapterError, ExecutionResolution, RunContext, Scope};
pub use signal::{
    AYNI_SIGNAL_SCHEMA_VERSION, AggregateStatus, AggregateSummary, AppliedThreshold, Budget,
    CommandFailure, CompletionIssue, CompletionScope, CompletionStage, CompletionState,
    ComplexityOffender, ComplexityResult, CoverageOffender, CoverageResult, DepsOffender,
    DepsResult, FailureSummary, Finding, FindingError, FindingMetadata, Findings,
    InvocationContext, Level, MutationOffender, MutationResult, OffenderIdentity, OffenderSummary,
    Offenders, OutputContext, RunArtifact, RunArtifactMetadata, RunCompletion, SignalKind,
    SignalResult, SignalRow, SizeOffender, SizeResult, TestFailure, TestResult,
    VerificationMetadata, VerificationTarget,
};
pub use threshold::classify_maximum;
