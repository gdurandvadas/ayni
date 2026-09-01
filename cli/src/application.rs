use ayni_core::{Language, SignalKind};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OutputFormat {
    Human,
    Json,
    Markdown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ExecutionMode {
    Managed,
    Host,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct CapabilityAuthorization {
    pub allow_network: bool,
    pub allow_docker_socket: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Operation {
    Init(InitOperation),
    EnvShow(EnvShowOperation),
    EnvDoctor(RepositoryOperation),
    EnvLock(EnvLockOperation),
    EnvBuild(RepositoryOperation),
    EnvStorage(EnvStorageOperation),
    EnvPrune(EnvPruneOperation),
    EnvShell(EnvShellOperation),
    EnvRun(EnvRunOperation),
    ContractShow(ContractOperation),
    Verify(VerifyOperation),
    VerifyList(VerifyListOperation),
    ImpactShow(ImpactOperation),
    ImpactRun(ImpactOperation),
    Check(CheckOperation),
    AgentsSync(RepositoryOperation),
    ResultsCompare(ResultsCompareOperation),
    GenerateDocs,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct InitOperation {
    pub repo_root: PathBuf,
    pub write: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RepositoryOperation {
    pub repo_root: PathBuf,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct EnvShowOperation {
    pub config: PathBuf,
    pub repo_root: PathBuf,
    pub output: OutputFormat,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct EnvLockOperation {
    pub config: PathBuf,
    pub repo_root: PathBuf,
    pub base: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct EnvStorageOperation {
    pub repo_root: PathBuf,
    pub output: OutputFormat,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct EnvPruneOperation {
    pub repo_root: PathBuf,
    pub output: OutputFormat,
    pub apply: bool,
    pub images: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct EnvShellOperation {
    pub repo_root: PathBuf,
    pub language: Option<Language>,
    pub root: Option<String>,
    pub authorization: CapabilityAuthorization,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct EnvRunOperation {
    pub repo_root: PathBuf,
    pub language: Option<Language>,
    pub root: Option<String>,
    pub command: Vec<String>,
    pub authorization: CapabilityAuthorization,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ContractOperation {
    pub config: PathBuf,
    pub output: OutputFormat,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct CheckOperation {
    pub config: PathBuf,
    pub output: OutputFormat,
    pub execution_mode: ExecutionMode,
    pub debug: bool,
    pub authorization: CapabilityAuthorization,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct VerifyListOperation {
    pub artifact: PathBuf,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct VerifyOperation {
    pub signal: SignalKind,
    pub config: PathBuf,
    pub language: Option<Language>,
    pub root: Option<String>,
    pub file: Option<String>,
    pub package: Option<String>,
    pub name: Option<String>,
    pub output: OutputFormat,
    pub execution_mode: ExecutionMode,
    pub debug: bool,
    pub authorization: CapabilityAuthorization,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ImpactOperation {
    pub config: PathBuf,
    pub base: String,
    pub output: OutputFormat,
    pub execution_mode: ExecutionMode,
    pub debug: bool,
    pub authorization: CapabilityAuthorization,
    /// Internal immutable host-produced impact plan for managed execution.
    pub managed_handoff: Option<PathBuf>,
    /// Internal provisional artifact path promoted by the managed outer process.
    pub managed_result: Option<PathBuf>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ResultsCompareOperation {
    pub baseline: PathBuf,
    pub candidate: PathBuf,
    pub output: OutputFormat,
}
