//! Adapter-owned exact environment resolution contracts.
use crate::{AdapterError, Language, TargetEnvironment};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct EnvironmentResolutionRequest {
    repo_root: PathBuf,
    target: TargetEnvironment,
}

impl EnvironmentResolutionRequest {
    pub fn new(repo_root: PathBuf, target: TargetEnvironment) -> Result<Self, AdapterError> {
        let language = target.target.language;
        if !repo_root.is_absolute() {
            return Err(AdapterError::new(
                language,
                "environment resolution repository root must be absolute",
            ));
        }
        Ok(Self { repo_root, target })
    }

    #[must_use]
    pub fn repo_root(&self) -> &Path {
        &self.repo_root
    }

    #[must_use]
    pub const fn target(&self) -> &TargetEnvironment {
        &self.target
    }
}

pub trait EnvironmentResolutionCapability: Send + Sync {
    fn language(&self) -> Language;
    fn resolve(
        &self,
        request: &EnvironmentResolutionRequest,
    ) -> Result<TargetEnvironment, AdapterError>;
}
