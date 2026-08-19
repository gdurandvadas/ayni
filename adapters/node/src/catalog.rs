use ayni_core::{CatalogEntry, SignalKind};

/// Tool catalog for the Node adapter.
///
/// These entries are declarative signal requirements; package-manager
/// resolution and dependency preparation remain separate adapter capabilities.
pub static NODE_CATALOG: &[CatalogEntry] = &[
    CatalogEntry {
        name: "node",
        for_signals: &[
            SignalKind::Test,
            SignalKind::Coverage,
            SignalKind::Size,
            SignalKind::Complexity,
            SignalKind::Deps,
        ],
        opt_in: false,
    },
    CatalogEntry {
        name: "vitest",
        for_signals: &[SignalKind::Test, SignalKind::Coverage],
        opt_in: false,
    },
    CatalogEntry {
        name: "@vitest/coverage-v8",
        for_signals: &[SignalKind::Coverage],
        opt_in: false,
    },
    CatalogEntry {
        name: "eslint",
        for_signals: &[SignalKind::Complexity],
        opt_in: false,
    },
    CatalogEntry {
        name: "@typescript-eslint/parser",
        for_signals: &[SignalKind::Complexity],
        opt_in: false,
    },
];

#[cfg(test)]
mod tests {
    use super::NODE_CATALOG;
    use ayni_core::SignalKind;

    #[test]
    fn complexity_requires_eslint_and_its_typescript_parser() {
        let tools = NODE_CATALOG
            .iter()
            .filter(|entry| entry.for_signals.contains(&SignalKind::Complexity))
            .map(|entry| entry.name)
            .collect::<Vec<_>>();
        assert_eq!(tools, ["node", "eslint", "@typescript-eslint/parser"]);
    }

    #[test]
    fn does_not_advertise_mutation_tools() {
        assert!(
            NODE_CATALOG
                .iter()
                .all(|entry| !entry.for_signals.contains(&SignalKind::Mutation))
        );
    }
}
