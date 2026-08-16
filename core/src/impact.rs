//! Typed, read-only impact planning contracts. These are intentionally separate
//! from the versioned `RunArtifact` schema: an impact plan is iteration advice,
//! never repository-completion evidence.

use crate::{AdapterError, Language, SignalKind, VerificationSelectorSupport};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed,
    Copied,
    TypeChanged,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ChangedPath {
    pub kind: ChangeKind,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_path: Option<String>,
}
impl ChangedPath {
    pub fn validate(&self) -> Result<(), ImpactError> {
        valid_relative_path(&self.path)?;
        if let Some(previous) = &self.previous_path {
            valid_relative_path(previous)?;
        }
        if self.kind == ChangeKind::Renamed && self.previous_path.is_none() {
            return Err(ImpactError::InvalidChange(
                "renamed change needs previous_path".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImpactConfidence {
    Certain,
    High,
    Medium,
    Low,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImpactReasonKind {
    ChangedFile,
    ChangedManifest,
    PackageOwnership,
    ReverseDependency,
    TopologyChanged,
    ContractChanged,
    EnvironmentChanged,
    UncertainOwnership,
    UnsupportedCapability,
    ConservativeBroadening,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ImpactReason {
    pub kind: ImpactReasonKind,
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImpactUncertaintyKind {
    UnknownPathOwnership,
    MissingTopology,
    AmbiguousOwnership,
    TopologyChanged,
    ConfigurationChanged,
    Unsupported,
    MetadataUnreadable,
}
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ImpactUncertainty {
    pub kind: ImpactUncertaintyKind,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SelectedCheck {
    pub language: Language,
    pub configured_root: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    pub signal: SignalKind,
    pub reasons: Vec<ImpactReason>,
    pub confidence: ImpactConfidence,
}
impl SelectedCheck {
    pub fn root(
        language: Language,
        configured_root: String,
        signal: SignalKind,
        reason: ImpactReason,
        confidence: ImpactConfidence,
    ) -> Self {
        Self {
            language,
            configured_root,
            package: None,
            file: None,
            signal,
            reasons: vec![reason],
            confidence,
        }
    }
    pub fn validate(&self) -> Result<(), ImpactError> {
        valid_root(&self.configured_root)?;
        if self.package.is_some() && self.file.is_some() {
            return Err(ImpactError::InvalidSelection(
                "cannot combine package and file".into(),
            ));
        }
        if let Some(file) = &self.file {
            valid_relative_path(file)?;
        }
        if self.reasons.is_empty()
            || self
                .reasons
                .iter()
                .any(|reason| reason.detail.trim().is_empty())
        {
            return Err(ImpactError::InvalidSelection(
                "selected check needs a non-empty reason".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImpactRequest {
    repo_root: PathBuf,
    pub language: Language,
    pub configured_root: String,
    pub changes: Vec<ChangedPath>,
    pub enabled_signals: BTreeSet<SignalKind>,
}
impl ImpactRequest {
    pub fn new(
        repo_root: PathBuf,
        language: Language,
        configured_root: String,
        mut changes: Vec<ChangedPath>,
        enabled_signals: impl IntoIterator<Item = SignalKind>,
    ) -> Result<Self, AdapterError> {
        if !repo_root.is_absolute() {
            return Err(AdapterError::new(
                language,
                "impact repository root must be absolute",
            ));
        }
        valid_root(&configured_root).map_err(|e| AdapterError::new(language, e.to_string()))?;
        changes.sort();
        changes.dedup();
        for change in &changes {
            change
                .validate()
                .map_err(|e| AdapterError::new(language, e.to_string()))?;
        }
        Ok(Self {
            repo_root,
            language,
            configured_root,
            changes,
            enabled_signals: enabled_signals.into_iter().collect(),
        })
    }
    pub fn repo_root(&self) -> &Path {
        &self.repo_root
    }
    pub fn configured_root_path(&self) -> PathBuf {
        if self.configured_root == "." {
            self.repo_root.clone()
        } else {
            self.repo_root.join(&self.configured_root)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImpactContribution {
    pub language: Language,
    pub configured_root: String,
    pub selected_checks: Vec<SelectedCheck>,
    pub uncertainties: Vec<ImpactUncertainty>,
}
impl ImpactContribution {
    pub fn normalize(&mut self) {
        for item in &mut self.selected_checks {
            item.reasons.sort();
            item.reasons.dedup();
        }
        self.selected_checks.sort();
        self.selected_checks.dedup_by(|current, retained| {
            if current.language == retained.language
                && current.configured_root == retained.configured_root
                && current.package == retained.package
                && current.file == retained.file
                && current.signal == retained.signal
            {
                retained.reasons.extend(current.reasons.clone());
                retained.reasons.sort();
                retained.reasons.dedup();
                // Keep the least-certain evidence when several changes select one check.
                retained.confidence = retained.confidence.max(current.confidence);
                true
            } else {
                false
            }
        });
        self.uncertainties.sort();
        self.uncertainties.dedup();
    }
    pub fn validate(
        &self,
        request: &ImpactRequest,
        support: impl Fn(SignalKind) -> VerificationSelectorSupport,
    ) -> Result<(), AdapterError> {
        self.validate_target(request)?;
        for selection in &self.selected_checks {
            validate_selection(selection, request, &support)?;
        }
        self.validate_uncertainty_broadening(request)
    }

    fn validate_target(&self, request: &ImpactRequest) -> Result<(), AdapterError> {
        if self.language == request.language && self.configured_root == request.configured_root {
            Ok(())
        } else {
            Err(AdapterError::new(
                request.language,
                "impact contribution language or configured root does not match request",
            ))
        }
    }

    fn validate_uncertainty_broadening(&self, request: &ImpactRequest) -> Result<(), AdapterError> {
        if self.uncertainties.is_empty()
            || request.enabled_signals.iter().all(|signal| {
                self.selected_checks.iter().any(|selection| {
                    selection.signal == *signal
                        && selection.package.is_none()
                        && selection.file.is_none()
                })
            })
        {
            Ok(())
        } else {
            Err(AdapterError::new(
                request.language,
                "impact uncertainty must broaden every enabled signal to configured root",
            ))
        }
    }
}

fn validate_selection(
    selection: &SelectedCheck,
    request: &ImpactRequest,
    support: &impl Fn(SignalKind) -> VerificationSelectorSupport,
) -> Result<(), AdapterError> {
    selection
        .validate()
        .map_err(|error| AdapterError::new(request.language, error.to_string()))?;
    if selection.language != request.language
        || selection.configured_root != request.configured_root
        || !request.enabled_signals.contains(&selection.signal)
    {
        return Err(AdapterError::new(
            request.language,
            "impact selection does not match the requested target and signals",
        ));
    }
    let selectors = support(selection.signal);
    if selection.file.is_some() && !selectors.file
        || selection.package.is_some() && !selectors.package
    {
        Err(AdapterError::new(
            request.language,
            "impact contribution uses an unsupported verification selector",
        ))
    } else {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImpactIdentityKind {
    Revision,
    WorkingTree,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ImpactIdentity {
    pub kind: ImpactIdentityKind,
    pub revision: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requested: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImpactPlan {
    pub base: ImpactIdentity,
    pub candidate: ImpactIdentity,
    pub changes: Vec<ChangedPath>,
    pub selected_checks: Vec<SelectedCheck>,
    pub uncertainties: Vec<ImpactUncertainty>,
    pub repository_completion_required: bool,
}
impl ImpactPlan {
    /// Canonicalize all independently accumulated plan rows before serialization.
    pub fn normalize(&mut self) {
        self.changes.sort();
        self.changes.dedup();
        let mut contribution = ImpactContribution {
            language: self
                .selected_checks
                .first()
                .map_or(Language::Rust, |check| check.language),
            configured_root: self
                .selected_checks
                .first()
                .map_or_else(|| String::from("."), |check| check.configured_root.clone()),
            selected_checks: std::mem::take(&mut self.selected_checks),
            uncertainties: std::mem::take(&mut self.uncertainties),
        };
        contribution.normalize();
        self.selected_checks = contribution.selected_checks;
        self.uncertainties = contribution.uncertainties;
    }
    pub fn validate(&self) -> Result<(), ImpactError> {
        if self.base.kind != ImpactIdentityKind::Revision
            || self.candidate.kind != ImpactIdentityKind::WorkingTree
            || self.base.revision.is_empty()
            || self.base.requested.as_deref().is_none_or(str::is_empty)
            || self.candidate.revision.is_empty()
            || self
                .candidate
                .fingerprint
                .as_deref()
                .is_none_or(str::is_empty)
        {
            return Err(ImpactError::InvalidPlan(
                "resolved base and fingerprinted working-tree candidate identities are required"
                    .into(),
            ));
        }
        if !self.repository_completion_required {
            return Err(ImpactError::InvalidPlan(
                "impact plans must explicitly require repository completion".into(),
            ));
        }
        for c in &self.changes {
            c.validate()?;
        }
        for s in &self.selected_checks {
            s.validate()?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImpactError {
    InvalidPath(String),
    InvalidChange(String),
    InvalidSelection(String),
    InvalidPlan(String),
}
impl std::fmt::Display for ImpactError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidPath(v)
            | Self::InvalidChange(v)
            | Self::InvalidSelection(v)
            | Self::InvalidPlan(v) => f.write_str(v),
        }
    }
}
impl std::error::Error for ImpactError {}
fn valid_relative_path(path: &str) -> Result<(), ImpactError> {
    let windows_prefix = path.as_bytes().get(1) == Some(&b':')
        && path.as_bytes().first().is_some_and(u8::is_ascii_alphabetic);
    let invalid_component = Path::new(path).components().any(|component| {
        matches!(
            component,
            std::path::Component::ParentDir
                | std::path::Component::RootDir
                | std::path::Component::Prefix(_)
        )
    });
    if path.is_empty()
        || path.contains('\0')
        || path.contains('\\')
        || windows_prefix
        || Path::new(path).is_absolute()
        || invalid_component
    {
        Err(ImpactError::InvalidPath(format!(
            "impact path must be normalized, non-empty, and repository-relative: {path}"
        )))
    } else {
        Ok(())
    }
}
fn valid_root(path: &str) -> Result<(), ImpactError> {
    if path == "." {
        Ok(())
    } else {
        valid_relative_path(path)
    }
}

pub trait ImpactCapability: Send + Sync {
    fn language(&self) -> Language;
    fn analyze(&self, request: &ImpactRequest) -> Result<ImpactContribution, AdapterError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn uncertainty_requires_root_broadening() {
        let request = ImpactRequest::new(
            std::env::current_dir().unwrap(),
            Language::Rust,
            ".".into(),
            vec![],
            [SignalKind::Test],
        )
        .unwrap();
        let c = ImpactContribution {
            language: Language::Rust,
            configured_root: ".".into(),
            selected_checks: vec![SelectedCheck {
                language: Language::Rust,
                configured_root: ".".into(),
                package: Some("a".into()),
                file: None,
                signal: SignalKind::Test,
                reasons: vec![ImpactReason {
                    kind: ImpactReasonKind::ChangedFile,
                    detail: "x".into(),
                }],
                confidence: ImpactConfidence::High,
            }],
            uncertainties: vec![ImpactUncertainty {
                kind: ImpactUncertaintyKind::MissingTopology,
                detail: "x".into(),
            }],
        };
        assert!(
            c.validate(&request, |_| VerificationSelectorSupport::new(
                false, true, false
            ))
            .is_err()
        );
    }

    #[test]
    fn normalization_keeps_least_certain_evidence_and_merges_reasons() {
        let check = |detail: &str, confidence| SelectedCheck {
            language: Language::Rust,
            configured_root: String::from("."),
            package: Some(String::from("core")),
            file: None,
            signal: SignalKind::Test,
            reasons: vec![ImpactReason {
                kind: ImpactReasonKind::ChangedFile,
                detail: detail.to_owned(),
            }],
            confidence,
        };
        let mut contribution = ImpactContribution {
            language: Language::Rust,
            configured_root: String::from("."),
            selected_checks: vec![
                check("a changed", ImpactConfidence::Certain),
                check("b changed", ImpactConfidence::Low),
            ],
            uncertainties: Vec::new(),
        };
        contribution.normalize();
        assert_eq!(contribution.selected_checks.len(), 1);
        assert_eq!(
            contribution.selected_checks[0].confidence,
            ImpactConfidence::Low
        );
        assert_eq!(contribution.selected_checks[0].reasons.len(), 2);
    }

    #[test]
    fn rejects_unsafe_paths_and_incomplete_identities() {
        for path in ["", "../escape", "C:/escape", "nested\\escape"] {
            assert!(
                ChangedPath {
                    kind: ChangeKind::Modified,
                    path: path.to_owned(),
                    previous_path: None,
                }
                .validate()
                .is_err(),
                "{path:?}"
            );
        }
        let plan = ImpactPlan {
            base: ImpactIdentity {
                kind: ImpactIdentityKind::Revision,
                revision: String::from("abc"),
                requested: None,
                fingerprint: None,
            },
            candidate: ImpactIdentity {
                kind: ImpactIdentityKind::WorkingTree,
                revision: String::from("def"),
                requested: None,
                fingerprint: Some(String::from("sha256:123")),
            },
            changes: Vec::new(),
            selected_checks: Vec::new(),
            uncertainties: Vec::new(),
            repository_completion_required: true,
        };
        assert!(plan.validate().is_err());
    }
}
