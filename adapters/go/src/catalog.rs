use ayni_core::{CatalogEntry, SignalKind};

/// Module and exact version used by managed environment provisioning.
pub(crate) const GOCYCLO_MODULE: &str = "github.com/fzipp/gocyclo/cmd/gocyclo";
pub(crate) const GOCYCLO_VERSION: &str = "0.6.0";

/// Tool catalog for the Go adapter.
pub static GO_CATALOG: &[CatalogEntry] = &[
    CatalogEntry {
        name: "go",
        for_signals: &[
            SignalKind::Test,
            SignalKind::Coverage,
            SignalKind::Size,
            SignalKind::Deps,
        ],
        opt_in: false,
    },
    CatalogEntry {
        name: "gocyclo",
        for_signals: &[SignalKind::Complexity],
        opt_in: false,
    },
];

#[cfg(test)]
mod tests {
    use super::GO_CATALOG;
    use ayni_core::SignalKind;

    #[test]
    fn does_not_advertise_mutation_tools() {
        assert!(
            GO_CATALOG
                .iter()
                .all(|entry| !entry.for_signals.contains(&SignalKind::Mutation))
        );
    }
}
