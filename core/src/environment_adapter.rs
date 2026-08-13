//! Adapter-facing environment discovery capability contracts.
//!
//! This module defines semantic inputs and outputs only. Language adapters own
//! repository-file interpretation; provider commands, filesystem mutation, and
//! provisioning execution remain outside core.

use crate::{
    AdapterError, EnvironmentContribution, Language, SignalKind, TargetIdentity, TargetPlatform,
};
use std::collections::BTreeSet;
use std::fs;
use std::io::ErrorKind;
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
        let repo_root = repo_root.canonicalize().map_err(|error| {
            AdapterError::new(
                target.language,
                format!(
                    "failed to establish environment discovery repository containment for {}: {error}",
                    repo_root.display()
                ),
            )
        })?;
        validate_target_containment(&repo_root, &target)?;
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

fn validate_target_containment(
    canonical_repo_root: &Path,
    target: &TargetIdentity,
) -> Result<(), AdapterError> {
    let candidate = canonical_repo_root.join(&target.root);
    let mut existing_ancestor = candidate.as_path();
    loop {
        match fs::symlink_metadata(existing_ancestor) {
            Ok(_) => break,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                existing_ancestor = existing_ancestor.parent().ok_or_else(|| {
                    AdapterError::new(
                        target.language,
                        format!(
                            "environment target '{}' has no existing repository ancestor",
                            target.root
                        ),
                    )
                })?;
            }
            Err(error) => {
                return Err(AdapterError::new(
                    target.language,
                    format!(
                        "environment target '{}' violates repository containment: cannot inspect {}: {error}",
                        target.root,
                        existing_ancestor.display()
                    ),
                ));
            }
        }
    }
    let resolved = existing_ancestor.canonicalize().map_err(|error| {
        AdapterError::new(
            target.language,
            format!(
                "environment target '{}' violates repository containment: cannot resolve {}: {error}",
                target.root,
                existing_ancestor.display()
            ),
        )
    })?;
    if !resolved.starts_with(canonical_repo_root) {
        return Err(AdapterError::new(
            target.language,
            format!(
                "environment target '{}' escapes repository containment: {} resolves outside {}",
                target.root,
                candidate.display(),
                canonical_repo_root.display()
            ),
        ));
    }
    if candidate.exists() && !resolved.is_dir() {
        return Err(AdapterError::new(
            target.language,
            format!(
                "environment target '{}' is not a directory: {}",
                target.root,
                candidate.display()
            ),
        ));
    }
    Ok(())
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
    use crate::{Architecture, Libc, OperatingSystem, SignalKind, TargetIdentity, TargetPlatform};
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn platform(architecture: Architecture) -> TargetPlatform {
        TargetPlatform {
            os: OperatingSystem::Linux,
            architecture,
            libc: Libc::Glibc,
        }
    }

    #[test]
    fn request_normalizes_platforms_signals_and_target_root() {
        let repository = TempDir::new().expect("repository");
        std::fs::create_dir_all(repository.path().join("apps/web")).expect("target root");
        let request = EnvironmentDiscoveryRequest::new(
            repository.path().to_path_buf(),
            TargetIdentity::new(crate::Language::Node, "apps/web").expect("target"),
            [SignalKind::Coverage, SignalKind::Test, SignalKind::Test],
            vec![
                platform(Architecture::Arm64),
                platform(Architecture::Amd64),
                platform(Architecture::Arm64),
            ],
        )
        .expect("request");

        assert_eq!(
            request.target_root(),
            repository
                .path()
                .canonicalize()
                .expect("canonical repository")
                .join("apps/web")
        );
        assert_eq!(request.enabled_signals().len(), 2);
        assert_eq!(
            request.requested_platforms(),
            [platform(Architecture::Amd64), platform(Architecture::Arm64)]
        );
    }

    #[test]
    fn tool_selection_uses_any_enabled_signal_semantics() {
        let repository = TempDir::new().expect("repository");
        let request = EnvironmentDiscoveryRequest::new(
            repository.path().to_path_buf(),
            TargetIdentity::new(crate::Language::Node, ".").expect("target"),
            [SignalKind::Coverage],
            vec![platform(Architecture::Amd64)],
        )
        .expect("request");

        assert!(request.requires_any(&[SignalKind::Test, SignalKind::Coverage]));
        assert!(!request.requires_any(&[SignalKind::Test, SignalKind::Mutation]));
        assert!(!request.requires_any(&[]));
    }

    #[test]
    fn request_rejects_relative_repository_and_empty_platforms() {
        let target = TargetIdentity::new(crate::Language::Rust, ".").expect("target");
        assert!(
            EnvironmentDiscoveryRequest::new(
                PathBuf::from("repo"),
                target.clone(),
                [],
                vec![platform(Architecture::Amd64)],
            )
            .is_err()
        );
        let repository = TempDir::new().expect("repository");
        assert!(
            EnvironmentDiscoveryRequest::new(
                repository.path().to_path_buf(),
                target,
                [],
                Vec::new(),
            )
            .is_err()
        );
    }

    #[test]
    fn request_rejects_existing_target_that_is_not_a_directory() {
        let repository = TempDir::new().expect("repository");
        std::fs::write(repository.path().join("target"), "not a directory").expect("target file");
        let error = EnvironmentDiscoveryRequest::new(
            repository.path().to_path_buf(),
            TargetIdentity::new(crate::Language::Rust, "target").expect("target"),
            [],
            vec![platform(Architecture::Amd64)],
        )
        .expect_err("file target");
        assert!(error.message.contains("is not a directory"));
    }

    #[test]
    fn request_normalizes_or_rejects_forged_target_identity() {
        let repository = TempDir::new().expect("repository");
        std::fs::create_dir(repository.path().join("apps")).expect("apps");
        let normalized = EnvironmentDiscoveryRequest::new(
            repository.path().to_path_buf(),
            TargetIdentity {
                language: crate::Language::Rust,
                root: String::from(r"apps\"),
            },
            [],
            vec![platform(Architecture::Amd64)],
        )
        .expect("normalizable target");
        assert_eq!(normalized.target().root, "apps");

        for root in ["apps/../apps", "/tmp"] {
            let error = EnvironmentDiscoveryRequest::new(
                repository.path().to_path_buf(),
                TargetIdentity {
                    language: crate::Language::Rust,
                    root: root.to_string(),
                },
                [],
                vec![platform(Architecture::Amd64)],
            )
            .expect_err("non-portable target");
            assert!(error.message.contains("target root"));
        }
    }

    #[cfg(unix)]
    #[test]
    fn request_canonicalizes_repository_and_rejects_symlink_escape() {
        use std::os::unix::fs::symlink;

        let fixture = TempDir::new().expect("fixture");
        let repository = fixture.path().join("repository");
        let outside = fixture.path().join("outside");
        std::fs::create_dir(&repository).expect("repository");
        std::fs::create_dir(&outside).expect("outside");
        symlink(&repository, fixture.path().join("repository-link")).expect("repository link");
        symlink(&outside, repository.join("escape")).expect("escape link");

        let root_request = EnvironmentDiscoveryRequest::new(
            fixture.path().join("repository-link"),
            TargetIdentity::new(crate::Language::Rust, ".").expect("target"),
            [],
            vec![platform(Architecture::Amd64)],
        )
        .expect("canonical repository");
        assert_eq!(
            root_request.repo_root(),
            repository.canonicalize().expect("canonical")
        );

        let error = EnvironmentDiscoveryRequest::new(
            repository,
            TargetIdentity::new(crate::Language::Rust, "escape").expect("target"),
            [],
            vec![platform(Architecture::Amd64)],
        )
        .expect_err("escaping target");
        assert!(error.message.contains("escapes repository containment"));
    }
}
