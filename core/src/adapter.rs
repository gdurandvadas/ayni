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
    fn collector(&self) -> &dyn SignalCollector;
}
