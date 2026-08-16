use ayni_core::{
    AdapterError, ImpactCapability, ImpactConfidence, ImpactContribution, ImpactReason,
    ImpactReasonKind, ImpactRequest, ImpactUncertainty, ImpactUncertaintyKind, Language,
    SelectedCheck,
};

pub struct GoImpactCapability;

impl ImpactCapability for GoImpactCapability {
    fn language(&self) -> Language {
        Language::Go
    }

    fn analyze(&self, request: &ImpactRequest) -> Result<ImpactContribution, AdapterError> {
        let mut contribution = ImpactContribution {
            language: Language::Go,
            configured_root: request.configured_root.clone(),
            selected_checks: Vec::new(),
            uncertainties: Vec::new(),
        };
        let relevant = request.changes.iter().any(|change| {
            change_paths(change).any(|path| {
                path_in_root(path, &request.configured_root)
                    || is_governing_ancestor_input(path, &request.configured_root)
            })
        });
        if relevant {
            contribution.uncertainties.push(ImpactUncertainty {
                kind: ImpactUncertaintyKind::MissingTopology,
                detail: String::from(
                    "Go package topology is not yet mapped; checks broaden to configured root",
                ),
            });
            for signal in &request.enabled_signals {
                contribution.selected_checks.push(SelectedCheck::root(
                    Language::Go,
                    request.configured_root.clone(),
                    *signal,
                    ImpactReason {
                        kind: ImpactReasonKind::ConservativeBroadening,
                        detail:
                            "go impact capability conservatively uses configured-root execution"
                                .into(),
                    },
                    ImpactConfidence::Medium,
                ));
            }
        }
        Ok(contribution)
    }
}

fn is_governing_ancestor_input(path: &str, configured_root: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    [
        "go.mod",
        "go.sum",
        "go.work",
        "go.work.sum",
        ".go-version",
        ".tool-versions",
    ]
    .contains(&name)
        && path_parent_governs_root(path, configured_root)
}

fn path_parent_governs_root(path: &str, configured_root: &str) -> bool {
    let parent = path.rsplit_once('/').map_or(".", |(parent, _)| parent);
    parent == "." || configured_root == parent || configured_root.starts_with(&format!("{parent}/"))
}

fn change_paths(change: &ayni_core::ChangedPath) -> impl Iterator<Item = &str> {
    std::iter::once(change.path.as_str()).chain(change.previous_path.as_deref())
}

fn path_in_root(path: &str, configured_root: &str) -> bool {
    configured_root == "."
        || path == configured_root
        || path.starts_with(&format!("{configured_root}/"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ayni_core::{ChangeKind, ChangedPath, SignalKind};

    #[test]
    fn relevant_change_is_deterministic_and_records_missing_topology() {
        let directory = tempfile::tempdir().expect("fixture");
        std::fs::create_dir_all(directory.path().join("service")).expect("root");
        let request = ImpactRequest::new(
            directory.path().canonicalize().expect("canonical"),
            Language::Go,
            String::from("service"),
            vec![ChangedPath {
                kind: ChangeKind::Modified,
                path: String::from("go.work.sum"),
                previous_path: None,
            }],
            [SignalKind::Test],
        )
        .expect("request");

        let contribution = ayni_adapters_common::impact::assert_impact_capability_conformance(
            &GoImpactCapability,
            &request,
        )
        .expect("impact");

        assert_eq!(contribution.selected_checks.len(), 1);
        assert_eq!(
            contribution.uncertainties[0].kind,
            ImpactUncertaintyKind::MissingTopology
        );
    }
}
