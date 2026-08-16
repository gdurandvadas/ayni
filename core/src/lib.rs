//! Core contracts for Ayni's unified signal model (foundations / pre-1.0).

pub mod adapter;
pub mod catalog;
pub mod comparison;
pub mod environment;
pub mod environment_adapter;
mod environment_lock;
pub mod environment_preparation;
pub mod environment_resolution;
pub mod finding;
pub mod impact;
pub mod impact_result;
pub mod language;
pub mod policy;
pub mod registry;
pub mod run_outcome;
pub mod runtime;
pub mod signal;
pub mod threshold;

pub use adapter::{
    ComplexityThresholdKind, DetectResult, DiscoveredRoot, LanguageAdapter, LanguageProfile,
    PolicyEffectivenessFacts, ProjectDiscovery, ProjectLayout, SignalCollector,
    VerificationSelection, VerificationSelectorSupport,
};
pub use catalog::CatalogEntry;
pub use comparison::{
    ARTIFACT_COMPARISON_SCHEMA_VERSION, ArtifactComparison, ArtifactComparisonError,
    FindingIdChanges, MatchedRowComparison, MetricChange, MetricValue, RowChangeSet, SignalRowKey,
    ValueChange, compare_artifacts,
};
pub use environment::{
    Architecture, DependencyLockRequirement, ENVIRONMENT_PLAN_SCHEMA_VERSION, EnvironmentConflict,
    EnvironmentContribution, EnvironmentPlan, EnvironmentPlanError, EnvironmentWarning, Libc,
    OperatingSystem, PackageManagerRequirement, ProvisioningSupport, RepositoryIdentity,
    RequirementConfidence, RequirementSource, ResolvedEnvironmentPlan, RuntimeRequirement,
    SignalToolRequirement, SystemRequirement, SystemRequirementKind, TargetEnvironment,
    TargetIdentity, TargetPlatform, ToolInstallationScope, VersionRequirement,
};
pub use environment_adapter::{EnvironmentCapability, EnvironmentDiscoveryRequest};
pub use environment_lock::{
    ENVIRONMENT_LOCK_SCHEMA_VERSION, EnvironmentLock, LockedDependencyLock, LockedPackageManager,
    LockedRepositoryIdentity, LockedRequirementSource, LockedRuntime, LockedSignalTool,
    LockedTargetEnvironment, ProvisioningBase,
};
pub use environment_preparation::{
    DependencyPreparationCapability, DependencyPreparationPlan, DependencyPreparationRequest,
    PreparationCommand, PreparationInput, PreparationOutput, PreparationOutputMode,
    PreparationScaffold,
};
pub use environment_resolution::{EnvironmentResolutionCapability, EnvironmentResolutionRequest};
pub use impact::{
    ChangeKind, ChangedPath, ImpactCapability, ImpactConfidence, ImpactContribution, ImpactError,
    ImpactIdentity, ImpactIdentityKind, ImpactPlan, ImpactReason, ImpactReasonKind, ImpactRequest,
    ImpactUncertainty, ImpactUncertaintyKind, SelectedCheck,
};
pub use impact_result::{
    IMPACT_SCHEMA_VERSION, ImpactAggregate, ImpactArtifact, ImpactExecution, ImpactExecutionIssue,
    ImpactExecutionState, RepositoryCompletionMarker,
};
pub use language::Language;
pub use policy::{
    AYNI_POLICY_FILE, AyniPolicy, ComplexityPolicy, ConcurrencyPolicy, CoveragePolicy, DepsPolicy,
    ExecutionPolicy, LanguageSelection, LanguageTooling, LanguageToolingOverrides, PolicyChecks,
    PolicyEffectivenessWarning, ReportPolicy, SizeThreshold, ThresholdFloat, ThresholdInt,
    ToolCommandOverride,
};
pub use registry::AdapterRegistry;
pub use run_outcome::RunOutcome;
pub use runtime::{AdapterError, AdapterErrorKind, ExecutionResolution, RunContext, Scope};
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
pub use threshold::{
    ConfiguredMetricEvaluation, classify_maximum, classify_minimum, evaluate_configured_metric,
};
