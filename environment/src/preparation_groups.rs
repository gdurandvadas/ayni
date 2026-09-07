//! Connected native ownership and input/output groups for dependency preparation.
use crate::{BackendError, preparation::preparation_digest};
use ayni_core::DependencyPreparationPlan;

pub(crate) struct PreparationGroup {
    pub id: String,
    pub plans: Vec<DependencyPreparationPlan>,
}

pub(crate) fn groups(
    plans: &[DependencyPreparationPlan],
) -> Result<Vec<PreparationGroup>, BackendError> {
    let mut ordered = plans.to_vec();
    ordered.sort_by(|left, right| left.target.cmp(&right.target));
    let mut groups: Vec<Vec<DependencyPreparationPlan>> = Vec::new();
    for plan in ordered {
        let mut joined = vec![plan];
        let mut index = 0;
        while index < groups.len() {
            if groups[index]
                .iter()
                .any(|left| joined.iter().any(|right| connected(left, right)))
            {
                joined.extend(groups.remove(index));
                // The newly joined plans may connect to an earlier group.
                index = 0;
            } else {
                index += 1;
            }
        }
        joined.sort_by(|left, right| left.target.cmp(&right.target));
        groups.push(joined);
    }
    let mut result = groups
        .into_iter()
        .map(|plans| {
            let digest = preparation_digest(&plans)?;
            Ok(PreparationGroup {
                id: digest.trim_start_matches("sha256:").to_owned(),
                plans,
            })
        })
        .collect::<Result<Vec<_>, BackendError>>()?;
    result.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(result)
}

fn connected(left: &DependencyPreparationPlan, right: &DependencyPreparationPlan) -> bool {
    let shared_owner = left.target.language == right.target.language
        && left
            .inputs
            .iter()
            .any(|a| right.inputs.iter().any(|b| a.owner_root == b.owner_root));
    shared_owner
        || shared_paths(left, right)
        || left.outputs.iter().any(|a| {
            right
                .outputs
                .iter()
                .any(|b| overlaps(&a.path, &b.path) || overlaps(&a.mount_path, &b.mount_path))
        })
}

fn shared_paths(left: &DependencyPreparationPlan, right: &DependencyPreparationPlan) -> bool {
    let left_paths = left
        .inputs
        .iter()
        .map(|input| &input.path)
        .chain(left.scaffolds.iter().map(|input| &input.path));
    let right_paths = right
        .inputs
        .iter()
        .map(|input| &input.path)
        .chain(right.scaffolds.iter().map(|input| &input.path))
        .collect::<Vec<_>>();
    left_paths
        .into_iter()
        .any(|left| right_paths.iter().any(|right| overlaps(left, right)))
}

fn overlaps(left: &str, right: &str) -> bool {
    left == "."
        || right == "."
        || left == right
        || left
            .strip_prefix(right)
            .is_some_and(|suffix| suffix.starts_with('/'))
        || right
            .strip_prefix(left)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ayni_core::{Language, PreparationInput, TargetIdentity};
    use std::collections::BTreeMap;

    fn plan(root: &str, owner: &str) -> DependencyPreparationPlan {
        DependencyPreparationPlan {
            target: TargetIdentity::new(Language::Node, root).unwrap(),
            inputs: vec![PreparationInput {
                path: format!("{root}/package.json"),
                digest: "sha256:input".into(),
                owner_root: owner.into(),
            }],
            commands: vec![],
            scaffolds: vec![],
            materialization_commands: vec![],
            outputs: vec![],
            execution_environment: BTreeMap::new(),
        }
    }

    #[test]
    fn groups_keep_workspace_owners_and_leave_independent_inputs_separate() {
        let first = plan("web/a", "web");
        let second = plan("web/b", "web");
        let independent = plan("service", "service");
        let grouped = groups(&[first.clone(), independent.clone(), second.clone()]).unwrap();
        assert_eq!(grouped.len(), 2);
        assert!(grouped.iter().any(|group| group.plans.len() == 2));
        let stable = groups(std::slice::from_ref(&independent))
            .unwrap()
            .remove(0)
            .id;
        assert!(grouped.iter().any(|group| group.id == stable));
        let mut changed = first;
        changed.inputs[0].digest = "sha256:changed".into();
        assert!(
            groups(&[changed, second, independent])
                .unwrap()
                .iter()
                .any(|group| group.id == stable)
        );
    }

    #[test]
    fn shared_inputs_connect_transitively_without_prefix_false_matches() {
        let a = plan("a", "a");
        let mut b = plan("b", "b");
        let mut c = plan("c", "c");
        b.inputs.push(a.inputs[0].clone());
        c.inputs.push(b.inputs[0].clone());
        assert_eq!(groups(&[a, b, c]).unwrap().len(), 1);
        assert!(!overlaps("web", "website"));
        assert!(overlaps("web/node_modules", "web/node_modules/pkg"));
    }
}
