//! Adapter-facing environment discovery capability contracts.
//!
//! This module defines semantic inputs and outputs only. Language adapters own
//! repository-file interpretation; provider commands, filesystem mutation, and
//! provisioning execution remain outside core.

use crate::{
    AdapterError, EnvironmentContribution, Language, SignalKind, TargetIdentity, TargetPlatform,
};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Read-only context supplied to one adapter environment capability.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvironmentDiscoveryRequest {
    repo_root: PathBuf,
    target: TargetIdentity,
    enabled_signals: BTreeSet<SignalKind>,
    requested_platforms: Vec<TargetPlatform>,
}

impl EnvironmentDiscoveryRequest {
    /// Constructs a lexical request from an already-contained repository root.
    ///
    /// Callers that obtain paths from the filesystem must validate canonical
    /// containment before calling this constructor.
    pub fn new(
        repo_root: PathBuf,
        target: TargetIdentity,
        enabled_signals: impl IntoIterator<Item = SignalKind>,
        mut requested_platforms: Vec<TargetPlatform>,
    ) -> Result<Self, AdapterError> {
        if !repo_root.is_absolute() {
            return Err(AdapterError::new(
                target.language,
                "environment discovery repository root must be absolute",
            ));
        }
        let target = TargetIdentity::new(target.language, &target.root)
            .map_err(|error| AdapterError::new(target.language, error.to_string()))?;
        if requested_platforms.is_empty() {
            return Err(AdapterError::new(
                target.language,
                "environment discovery needs a requested platform",
            ));
        }
        requested_platforms.sort();
        requested_platforms.dedup();
        Ok(Self {
            repo_root,
            target,
            enabled_signals: enabled_signals.into_iter().collect(),
            requested_platforms,
        })
    }

    #[must_use]
    pub fn repo_root(&self) -> &Path {
        &self.repo_root
    }

    #[must_use]
    pub const fn target(&self) -> &TargetIdentity {
        &self.target
    }

    #[must_use]
    pub fn target_root(&self) -> PathBuf {
        if self.target.root == "." {
            self.repo_root.clone()
        } else {
            self.repo_root.join(&self.target.root)
        }
    }

    #[must_use]
    pub fn enabled_signals(&self) -> &BTreeSet<SignalKind> {
        &self.enabled_signals
    }

    #[must_use]
    pub fn requested_platforms(&self) -> &[TargetPlatform] {
        &self.requested_platforms
    }

    #[must_use]
    pub fn requires_any(&self, signals: &[SignalKind]) -> bool {
        !signals.is_empty()
            && signals
                .iter()
                .any(|signal| self.enabled_signals.contains(signal))
    }
}

/// Separable, read-only environment capability implemented by a language
/// adapter. Implementations inspect repository inputs and return typed semantic
/// data; they do not provision tools, run quality collectors, or mutate files.
pub trait EnvironmentCapability: Send + Sync {
    fn language(&self) -> Language;

    fn discover(
        &self,
        request: &EnvironmentDiscoveryRequest,
    ) -> Result<EnvironmentContribution, AdapterError>;
}

pub(crate) fn validate_environment_contribution(
    language: Language,
    request: &EnvironmentDiscoveryRequest,
    contribution: &EnvironmentContribution,
) -> Result<(), AdapterError> {
    if request.target.language != language {
        return Err(AdapterError::new(
            language,
            "environment request language does not match capability language",
        ));
    }
    if contribution.target().target != *request.target() {
        return Err(AdapterError::new(
            language,
            "environment contribution target does not match the requested target",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::EnvironmentDiscoveryRequest;
    use crate::{
        Architecture, Language, Libc, OperatingSystem, SignalKind, TargetIdentity, TargetPlatform,
    };
    use std::path::PathBuf;

    fn platform(architecture: Architecture) -> TargetPlatform {
        TargetPlatform {
            os: OperatingSystem::Linux,
            architecture,
            libc: Libc::Glibc,
        }
    }

    fn absolute_root() -> PathBuf {
        std::env::current_dir().expect("current directory")
    }

    #[test]
    fn lexical_request_normalizes_platforms_signals_and_target_root() {
        let repository = absolute_root();
        let request = EnvironmentDiscoveryRequest::new(
            repository.clone(),
            TargetIdentity::new(Language::Node, "apps/web").expect("target"),
            [SignalKind::Coverage, SignalKind::Test, SignalKind::Test],
            vec![
                platform(Architecture::Arm64),
                platform(Architecture::Amd64),
                platform(Architecture::Arm64),
            ],
        )
        .expect("request");

        assert_eq!(request.target_root(), repository.join("apps/web"));
        assert_eq!(request.enabled_signals().len(), 2);
        assert_eq!(
            request.requested_platforms(),
            [platform(Architecture::Amd64), platform(Architecture::Arm64)]
        );
    }

    #[test]
    fn lexical_request_rejects_relative_repository_and_empty_platforms() {
        let target = TargetIdentity::new(Language::Rust, ".").expect("target");
        assert!(
            EnvironmentDiscoveryRequest::new(
                PathBuf::from("repo"),
                target.clone(),
                [],
                vec![platform(Architecture::Amd64)],
            )
            .is_err()
        );
        assert!(
            EnvironmentDiscoveryRequest::new(absolute_root(), target, [], Vec::new(),).is_err()
        );
    }

    #[test]
    fn lexical_request_normalizes_or_rejects_forged_target_identity() {
        let normalized = EnvironmentDiscoveryRequest::new(
            absolute_root(),
            TargetIdentity {
                language: Language::Rust,
                root: String::from(r"apps\"),
            },
            [],
            vec![platform(Architecture::Amd64)],
        )
        .expect("normalizable target");
        assert_eq!(normalized.target().root, "apps");

        for root in ["apps/../apps", "/tmp"] {
            let error = EnvironmentDiscoveryRequest::new(
                absolute_root(),
                TargetIdentity {
                    language: Language::Rust,
                    root: root.to_string(),
                },
                [],
                vec![platform(Architecture::Amd64)],
            )
            .expect_err("non-portable target");
            assert!(error.message.contains("target root"));
        }
    }
}
