use ayni_core::{CatalogEntry, SignalKind};

/// Exact managed-environment version for the coverage tool.
pub const CARGO_LLVM_COV_VERSION: &str = "0.8.5";

/// Tool catalog for the Rust adapter.
///
/// Entries identify the tools each signal requires. Managed-environment
/// provisioning is defined by the environment capability, not this catalog.
pub static RUST_CATALOG: &[CatalogEntry] = &[
    CatalogEntry {
        name: "cargo",
        for_signals: &[SignalKind::Test],
        opt_in: false,
    },
    CatalogEntry {
        name: "llvm-tools-preview",
        for_signals: &[SignalKind::Coverage],
        opt_in: false,
    },
    CatalogEntry {
        name: "cargo-llvm-cov",
        for_signals: &[SignalKind::Coverage],
        opt_in: false,
    },
    CatalogEntry {
        name: "rust-code-analysis-cli",
        for_signals: &[SignalKind::Complexity],
        opt_in: false,
    },
];

#[cfg(test)]
mod tests {
    use super::RUST_CATALOG;
    use ayni_core::SignalKind;

    #[test]
    fn does_not_advertise_mutation_tools() {
        assert!(
            RUST_CATALOG
                .iter()
                .all(|entry| !entry.for_signals.contains(&SignalKind::Mutation))
        );
    }
}
