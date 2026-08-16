use ayni_core::{CatalogEntry, SignalKind};

/// Tool catalog for the Python adapter.
///
/// These entries are declarative signal requirements; package-manager
/// resolution and dependency preparation remain separate adapter capabilities.
pub static PYTHON_CATALOG: &[CatalogEntry] = &[
    CatalogEntry {
        name: "python",
        for_signals: &[
            SignalKind::Test,
            SignalKind::Coverage,
            SignalKind::Size,
            SignalKind::Complexity,
            SignalKind::Deps,
            SignalKind::Mutation,
        ],
        opt_in: false,
    },
    CatalogEntry {
        name: "pytest",
        for_signals: &[SignalKind::Test, SignalKind::Coverage],
        opt_in: false,
    },
    CatalogEntry {
        name: "pytest-json-report",
        for_signals: &[SignalKind::Test],
        opt_in: false,
    },
    CatalogEntry {
        name: "pytest-cov",
        for_signals: &[SignalKind::Coverage],
        opt_in: false,
    },
    CatalogEntry {
        name: "coverage",
        for_signals: &[SignalKind::Coverage],
        opt_in: false,
    },
    CatalogEntry {
        name: "complexipy",
        for_signals: &[SignalKind::Complexity],
        opt_in: false,
    },
    CatalogEntry {
        name: "mutmut",
        for_signals: &[SignalKind::Mutation],
        opt_in: true,
    },
];
