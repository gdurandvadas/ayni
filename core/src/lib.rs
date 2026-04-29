//! Core contracts for Ayni's unified signal model (foundations / pre-1.0).

pub mod adapter;
pub mod catalog;
pub mod language;
pub mod policy;
pub mod registry;
pub mod runtime;
pub mod signal;

pub use adapter::{DetectResult, LanguageAdapter, LanguageProfile, SignalCollector};
pub use catalog::{
    CatalogEntry, InstallContext, Installer, NodePackageManager, ToolStatus, VersionCheck,
    detect_node_package_manager,
};
pub use language::Language;
pub use policy::{
    AYNI_POLICY_FILE, AyniPolicy, ComplexityPolicy, ConcurrencyPolicy, CoveragePolicy, DepsPolicy,
    LanguageSelection, LanguageTooling, LanguageToolingOverrides, PolicyChecks, ReportPolicy,
    SizeThreshold, ThresholdFloat, ThresholdInt, ToolCommandOverride,
};
pub use registry::AdapterRegistry;
pub use runtime::{AdapterError, BranchDiff, RunContext, Scope};
pub use signal::{
    AYNI_SIGNAL_SCHEMA_VERSION, Budget, ComplexityOffender, ComplexityResult, CoverageOffender,
    CoverageResult, Delta, DepsOffender, DepsResult, Level, MutationOffender, MutationResult,
    Offenders, RunArtifact, Severity, SignalKind, SignalResult, SignalRow, SizeOffender,
    SizeResult, TestFailure, TestResult,
};
