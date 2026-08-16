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

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Operation {
    EnvShow(EnvShowOperation),
    EnvDoctor(RepositoryOperation),
    EnvLock(EnvLockOperation),
    EnvBuild(RepositoryOperation),
    EnvShell(EnvShellOperation),
    EnvRun(EnvRunOperation),
    ContractShow(ContractOperation),
    ContractValidate(ContractOperation),
    Verify(VerifyOperation),
    ImpactShow(ImpactOperation),
    ImpactRun(ImpactOperation),
    Check(CheckOperation),
    AgentsSync(RepositoryOperation),
    ResultsCompare(ResultsCompareOperation),
    GenerateDocs,
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
pub(crate) struct EnvShellOperation {
    pub repo_root: PathBuf,
    pub language: Option<Language>,
    pub root: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct EnvRunOperation {
    pub repo_root: PathBuf,
    pub language: Option<Language>,
    pub root: Option<String>,
    pub command: Vec<String>,
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
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ImpactOperation {
    pub config: PathBuf,
    pub base: String,
    pub output: OutputFormat,
    pub execution_mode: ExecutionMode,
    pub debug: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ResultsCompareOperation {
    pub baseline: PathBuf,
    pub candidate: PathBuf,
    pub output: OutputFormat,
}
