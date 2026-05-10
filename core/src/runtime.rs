use crate::catalog::PythonPackageManagerResolution;
use crate::language::Language;
use crate::policy::AyniPolicy;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Scope {
    pub workspace_root: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct BranchDiff {
    #[serde(default)]
    pub merge_base: Option<String>,
    #[serde(default)]
    pub changed_paths: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct RunContext {
    pub repo_root: PathBuf,
    pub workdir: PathBuf,
    pub policy: AyniPolicy,
    pub scope: Scope,
    pub diff: Option<BranchDiff>,
    pub python_resolution: Option<PythonPackageManagerResolution>,
    pub debug: bool,
}

#[derive(Debug, Clone)]
pub struct AdapterError {
    pub language: Language,
    pub message: String,
}

impl AdapterError {
    #[must_use]
    pub fn new(language: Language, message: impl Into<String>) -> Self {
        Self {
            language,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for AdapterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} adapter error: {}", self.language, self.message)
    }
}

impl std::error::Error for AdapterError {}
