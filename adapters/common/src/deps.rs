//! Shared forbidden dependency rules; adapters supply their own scoped edges.
use ayni_core::{DepsOffender, Level};
use glob::Pattern;
use std::collections::{BTreeMap, BTreeSet};

pub struct CompiledRule {
    from_raw: String,
    to_raw: String,
    from: Pattern,
    to: Pattern,
}

pub fn compile_rules(
    forbidden: &BTreeMap<String, Vec<String>>,
) -> Result<Vec<CompiledRule>, String> {
    let mut compiled = Vec::new();
    for (from, tos) in forbidden {
        let from_pattern = Pattern::new(from)
            .map_err(|error| format!("invalid forbidden deps pattern '{from}': {error}"))?;
        for to in tos {
            compiled.push(CompiledRule {
                from_raw: from.clone(),
                to_raw: to.clone(),
                from: from_pattern.clone(),
                to: Pattern::new(to)
                    .map_err(|error| format!("invalid forbidden deps pattern '{to}': {error}"))?,
            });
        }
    }
    Ok(compiled)
}

pub fn matching_offenders(
    edges: &BTreeSet<(String, String)>,
    compiled_rules: &[CompiledRule],
) -> Vec<DepsOffender> {
    let mut offenders = Vec::new();
    for (from, to) in edges {
        for rule in compiled_rules {
            if rule.from.matches(from) && rule.to.matches(to) {
                offenders.push(DepsOffender {
                    from: from.clone(),
                    to: to.clone(),
                    rule: format!("{} -> {}", rule.from_raw, rule.to_raw),
                    level: Level::Fail,
                });
            }
        }
    }

    offenders.sort_by(|left, right| {
        left.from
            .cmp(&right.from)
            .then_with(|| left.to.cmp(&right.to))
            .then_with(|| left.rule.cmp(&right.rule))
    });

    offenders
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retains_overlapping_rules_and_orders_offenders() {
        let rules = BTreeMap::from([
            (
                String::from("apps/*"),
                vec![String::from("libs/*"), String::from("libs/private")],
            ),
            (String::from("apps/api"), vec![String::from("libs/private")]),
        ]);
        let edges = BTreeSet::from([
            (String::from("apps/api"), String::from("libs/private")),
            (String::from("libs/public"), String::from("libs/private")),
        ]);
        let offenders = matching_offenders(&edges, &compile_rules(&rules).unwrap());
        assert_eq!(offenders.len(), 3);
        assert_eq!(
            offenders
                .iter()
                .map(|item| item.rule.as_str())
                .collect::<Vec<_>>(),
            vec![
                "apps/* -> libs/*",
                "apps/* -> libs/private",
                "apps/api -> libs/private"
            ]
        );
        assert!(offenders.iter().all(|item| item.level == Level::Fail));
        assert!(matching_offenders(&edges, &[]).is_empty());
    }

    #[test]
    fn rejects_invalid_source_and_destination_globs() {
        for (from, to) in [("[", "valid"), ("valid", "[")] {
            let error = compile_rules(&BTreeMap::from([(from.into(), vec![to.into()])]))
                .err()
                .expect("invalid glob");
            assert!(error.contains("invalid forbidden deps pattern '['"));
        }
        assert!(compile_rules(&BTreeMap::from([(String::from("["), vec![])])).is_err());
    }
}
