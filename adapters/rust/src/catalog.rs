use ayni_core::{CatalogEntry, Installer, SignalKind, VersionCheck};

/// Tool catalog for the Rust adapter.
///
/// Drives `ayni install --language rust`. Every external tool the adapter invokes
/// to collect signals must be declared here. Order matters: `llvm-tools-preview`
/// must be installed before `cargo-llvm-cov`.
pub static RUST_CATALOG: &[CatalogEntry] = &[
    CatalogEntry {
        name: "cargo",
        check: Some(VersionCheck {
            command: "cargo",
            args: &["--version"],
            contains: None,
        }),
        installer: Installer::Bundled,
        for_signals: &[SignalKind::Test],
        opt_in: false,
    },
    CatalogEntry {
        name: "llvm-tools-preview",
        check: None,
        installer: Installer::Rustup {
            component: "llvm-tools-preview",
        },
        for_signals: &[SignalKind::Coverage],
        opt_in: false,
    },
    CatalogEntry {
        name: "cargo-llvm-cov",
        check: Some(VersionCheck {
            command: "cargo",
            args: &["llvm-cov", "--version"],
            contains: Some("0.8.5"),
        }),
        installer: Installer::Cargo {
            crate_name: "cargo-llvm-cov",
            version: Some("0.8.5"),
        },
        for_signals: &[SignalKind::Coverage],
        opt_in: false,
    },
    CatalogEntry {
        name: "rust-code-analysis-cli",
        check: Some(VersionCheck {
            command: "rust-code-analysis-cli",
            args: &["--version"],
            contains: None,
        }),
        installer: Installer::Cargo {
            crate_name: "rust-code-analysis-cli",
            version: None,
        },
        for_signals: &[SignalKind::Complexity],
        opt_in: false,
    },
    CatalogEntry {
        name: "cargo-mutants",
        check: Some(VersionCheck {
            command: "cargo",
            args: &["mutants", "--version"],
            contains: None,
        }),
        installer: Installer::Cargo {
            crate_name: "cargo-mutants",
            version: None,
        },
        for_signals: &[SignalKind::Mutation],
        opt_in: true,
    },
];
