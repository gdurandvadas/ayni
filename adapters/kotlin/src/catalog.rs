use ayni_core::{CatalogEntry, SignalKind};

/// Tool catalog for the Kotlin adapter.
pub static KOTLIN_CATALOG: &[CatalogEntry] = &[
    CatalogEntry {
        name: "gradle-test",
        for_signals: &[SignalKind::Test],
        opt_in: false,
    },
    CatalogEntry {
        name: "coverage-report",
        for_signals: &[SignalKind::Coverage],
        opt_in: false,
    },
    CatalogEntry {
        name: "detekt",
        for_signals: &[SignalKind::Complexity],
        opt_in: false,
    },
    CatalogEntry {
        name: "pitest",
        for_signals: &[SignalKind::Mutation],
        opt_in: true,
    },
];
