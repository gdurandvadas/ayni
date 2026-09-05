//! Read-only contracts for adapter-owned signal-tool reconciliation.
//!
//! Core validates semantic plans. It never reads repository files, executes a
//! package manager, or applies edits. A future executor must independently check
//! canonical containment and preimages before staging or publishing outputs.

use crate::{
    AdapterError, Language, SignalKind, TargetIdentity, ToolInstallationScope, VersionRequirement,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignalToolOwnership {
    #[default]
    Project,
    Ayni,
}

/// Who selects the tool version, independently of the evidence source path.
/// ProjectLocked includes declarations awaiting native-lock resolution; exact
/// resolution must not silently turn project authority into adapter authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolVersionAuthority {
    AdapterPinned,
    ProjectLocked,
    LockResolved,
    Toolchain,
}

impl ToolVersionAuthority {
    pub(crate) fn validate(self, scope: ToolInstallationScope, exact: bool) -> Result<(), String> {
        match self {
            Self::AdapterPinned if !exact => {
                Err("adapter-pinned tools require an exact baseline".into())
            }
            Self::ProjectLocked if scope != ToolInstallationScope::Project => {
                Err("project-locked tools require project installation scope".into())
            }
            Self::LockResolved if scope == ToolInstallationScope::Project => {
                Err("project tools must retain project or adapter version authority".into())
            }
            Self::Toolchain if scope != ToolInstallationScope::Runtime => {
                Err("toolchain tools require runtime installation scope".into())
            }
            _ => Ok(()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolingRequest {
    repo_root: PathBuf,
    target: TargetIdentity,
    enabled_signals: BTreeSet<SignalKind>,
    ownership: SignalToolOwnership,
    default_tool_signals: BTreeSet<SignalKind>,
}

impl ToolingRequest {
    /// `repo_root` must already have passed filesystem containment checks.
    /// `default_tool_signals` excludes signals using custom command overrides.
    pub fn new(
        repo_root: PathBuf,
        target: TargetIdentity,
        enabled_signals: impl IntoIterator<Item = SignalKind>,
        ownership: SignalToolOwnership,
        default_tool_signals: impl IntoIterator<Item = SignalKind>,
    ) -> Result<Self, AdapterError> {
        let error = |message| AdapterError::new(target.language, message);
        if !repo_root.is_absolute() {
            return Err(error("tooling repository root must be absolute"));
        }
        let enabled_signals = enabled_signals.into_iter().collect::<BTreeSet<_>>();
        let default_tool_signals = default_tool_signals.into_iter().collect::<BTreeSet<_>>();
        if !default_tool_signals.is_subset(&enabled_signals) {
            return Err(error("default tool signals must be enabled signals"));
        }
        let target = TargetIdentity::new(target.language, &target.root)
            .map_err(|cause| AdapterError::new(target.language, cause.to_string()))?;
        Ok(Self {
            repo_root,
            target,
            enabled_signals,
            ownership,
            default_tool_signals,
        })
    }

    pub fn repo_root(&self) -> &Path {
        &self.repo_root
    }
    pub fn target(&self) -> &TargetIdentity {
        &self.target
    }
    pub fn enabled_signals(&self) -> &BTreeSet<SignalKind> {
        &self.enabled_signals
    }
    pub fn ownership(&self) -> SignalToolOwnership {
        self.ownership
    }
    pub fn default_tool_signals(&self) -> &BTreeSet<SignalKind> {
        &self.default_tool_signals
    }
}

/// Absent is an explicit create-if-missing precondition, never an unchecked write.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ToolingPreimage {
    Absent,
    Sha256 { digest: String },
}

/// Existing metadata approved for copying into staging, including read-only inputs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolingInput {
    pub path: String,
    pub digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolingFileEdit {
    pub path: String,
    pub preimage: ToolingPreimage,
    pub content: String,
}

/// A repository file approved for copy-back. Directory-wide outputs are forbidden.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolingOutput {
    pub path: String,
    pub preimage: ToolingPreimage,
}

/// Structured argv, not shell text. The future executor must resolve the exact
/// adapter-approved executable; a bare name does not authorize ambient PATH use.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolingCommand {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: String,
    pub environment: BTreeMap<String, String>,
    /// Exact files this command may produce for copy-back, relative to the repo.
    pub outputs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolingRequirement {
    pub tool: String,
    pub baseline: VersionRequirement,
    pub authority: ToolVersionAuthority,
    pub scope: ToolInstallationScope,
    pub signals: BTreeSet<SignalKind>,
    pub declaration_path: Option<String>,
    pub current_declaration: Option<String>,
    pub current_resolution: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolingDiagnostic {
    pub code: String,
    pub message: String,
    pub path: Option<String>,
}

pub type ToolingWarning = ToolingDiagnostic;
pub type ToolingConflict = ToolingDiagnostic;

/// A proposal, not permission to execute. Public fields support adapter assembly;
/// the LanguageAdapter wrapper revalidates the entire returned plan. There is no
/// unchecked deserializer for this execution-sensitive boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ToolingPlan {
    pub target: TargetIdentity,
    pub owner_root: String,
    pub tools: Vec<ToolingRequirement>,
    pub inputs: Vec<ToolingInput>,
    pub edits: Vec<ToolingFileEdit>,
    pub commands: Vec<ToolingCommand>,
    pub outputs: Vec<ToolingOutput>,
    pub warnings: Vec<ToolingWarning>,
    pub conflicts: Vec<ToolingConflict>,
}

impl ToolingPlan {
    pub fn empty(target: TargetIdentity) -> Self {
        Self {
            owner_root: target.root.clone(),
            target,
            tools: Vec::new(),
            inputs: Vec::new(),
            edits: Vec::new(),
            commands: Vec::new(),
            outputs: Vec::new(),
            warnings: Vec::new(),
            conflicts: Vec::new(),
        }
    }

    pub fn normalize_and_validate(&mut self, request: &ToolingRequest) -> Result<(), AdapterError> {
        crate::tooling_validation::validate_plan(self, request)
            .map_err(|cause| AdapterError::new(request.target.language, cause))
    }
}

/// Planning must not mutate files or execute package-manager commands.
pub trait ToolingReconciliationCapability: Send + Sync {
    fn language(&self) -> Language;
    fn plan(&self, request: &ToolingRequest) -> Result<ToolingPlan, AdapterError>;
}

#[cfg(test)]
#[path = "tooling_tests.rs"]
mod tests;
