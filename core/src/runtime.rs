use crate::language::Language;
use crate::policy::AyniPolicy;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
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

#[derive(Debug, Clone)]
pub struct RunContext {
    pub repo_root: PathBuf,
    pub target_root: PathBuf,
    pub workdir: PathBuf,
    pub policy: AyniPolicy,
    pub scope: Scope,
    pub execution: ExecutionResolution,
    pub debug: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionResolution {
    pub runner: String,
    pub resolved_from: PathBuf,
    pub kind: String,
    pub source: String,
    pub confidence: u8,
    pub ambiguous: bool,
    pub install_cwd: PathBuf,
    pub exec_cwd: PathBuf,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub environment: BTreeMap<String, String>,
}

impl ExecutionResolution {
    #[must_use]
    pub fn direct(
        runner: impl Into<String>,
        root: PathBuf,
        source: impl Into<String>,
        confidence: u8,
    ) -> Self {
        Self {
            runner: runner.into(),
            resolved_from: root.clone(),
            kind: String::from("direct_root"),
            source: source.into(),
            confidence,
            ambiguous: false,
            install_cwd: root.clone(),
            exec_cwd: root,
            environment: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdapterErrorKind {
    Environment,
    Execution,
}

#[derive(Debug, Clone)]
pub struct AdapterError {
    pub language: Language,
    pub kind: AdapterErrorKind,
    pub message: String,
}

impl AdapterError {
    #[must_use]
    pub fn new(language: Language, message: impl Into<String>) -> Self {
        Self {
            language,
            kind: AdapterErrorKind::Environment,
            message: message.into(),
        }
    }

    #[must_use]
    pub fn execution(language: Language, message: impl Into<String>) -> Self {
        Self {
            language,
            kind: AdapterErrorKind::Execution,
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
