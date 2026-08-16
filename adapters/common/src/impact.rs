//! Shared conformance assertions for read-only impact capabilities.
use crate::environment::snapshot;
use ayni_core::{AdapterError, ImpactCapability, ImpactContribution, ImpactRequest};

/// Exercise an impact capability twice and require canonical, read-only output.
/// The caller owns fixture construction; this helper deliberately does not
/// interpret ecosystem metadata or execute commands.
pub fn assert_impact_capability_conformance(
    capability: &dyn ImpactCapability,
    request: &ImpactRequest,
) -> Result<ImpactContribution, AdapterError> {
    if capability.language() != request.language {
        return Err(AdapterError::new(
            request.language,
            "impact capability language mismatch",
        ));
    }
    let before = snapshot(request.repo_root()).map_err(|error| {
        AdapterError::new(
            request.language,
            format!("failed to snapshot impact fixture: {error}"),
        )
    })?;
    let mut first = capability.analyze(request)?;
    first.normalize();
    let after_first = snapshot(request.repo_root()).map_err(|error| {
        AdapterError::new(
            request.language,
            format!("failed to snapshot impact fixture after planning: {error}"),
        )
    })?;
    if before != after_first {
        return Err(AdapterError::new(
            request.language,
            "impact analysis mutated the repository",
        ));
    }
    let mut second = capability.analyze(request)?;
    second.normalize();
    let after_second = snapshot(request.repo_root()).map_err(|error| {
        AdapterError::new(
            request.language,
            format!("failed to snapshot impact fixture after repeated planning: {error}"),
        )
    })?;
    if before != after_second {
        return Err(AdapterError::new(
            request.language,
            "impact analysis mutated the repository",
        ));
    }
    if first != second {
        return Err(AdapterError::new(
            request.language,
            "impact capability is not deterministic",
        ));
    }
    Ok(first)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ayni_core::{
        ImpactConfidence, ImpactReason, ImpactReasonKind, Language, SelectedCheck, SignalKind,
    };
    struct Stable;
    impl ImpactCapability for Stable {
        fn language(&self) -> Language {
            Language::Rust
        }
        fn analyze(&self, request: &ImpactRequest) -> Result<ImpactContribution, AdapterError> {
            Ok(ImpactContribution {
                language: Language::Rust,
                configured_root: request.configured_root.clone(),
                selected_checks: vec![SelectedCheck::root(
                    Language::Rust,
                    request.configured_root.clone(),
                    SignalKind::Test,
                    ImpactReason {
                        kind: ImpactReasonKind::ChangedFile,
                        detail: "fixture".into(),
                    },
                    ImpactConfidence::High,
                )],
                uncertainties: vec![],
            })
        }
    }
    #[test]
    fn checks_deterministic_read_only_contributions() {
        let request = ImpactRequest::new(
            std::env::current_dir().unwrap(),
            Language::Rust,
            ".".into(),
            vec![],
            [SignalKind::Test],
        )
        .unwrap();
        assert!(assert_impact_capability_conformance(&Stable, &request).is_ok());
    }
}
