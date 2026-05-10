use crate::catalog::CatalogEntry;
use crate::language::Language;
use crate::runtime::{AdapterError, ExecutionResolution, RunContext};
use crate::signal::{SignalKind, SignalRow};
use serde::{Deserialize, Serialize};
use std::path::Path;

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

pub trait SignalCollector: Send + Sync {
    fn collect(&self, kind: SignalKind, context: &RunContext) -> Result<SignalRow, AdapterError>;
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
    fn profile(&self) -> LanguageProfile;
    fn catalog(&self) -> &'static [CatalogEntry];
    fn collector(&self) -> &dyn SignalCollector;
}
