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
            SignalKind::Mutation,
        ],
        opt_in: false,
    },
    CatalogEntry {
        name: "gocyclo",
        for_signals: &[SignalKind::Complexity],
        opt_in: false,
    },
];
