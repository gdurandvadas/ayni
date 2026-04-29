use ayni_core::{CatalogEntry, Installer, SignalKind, VersionCheck};

/// Tool catalog for the Node adapter.
///
/// Uses local dev dependencies in each detected Node root so mixed package
/// managers and per-root toolchains are supported.
pub static NODE_CATALOG: &[CatalogEntry] = &[
    CatalogEntry {
        name: "node",
        check: Some(VersionCheck {
            command: "node",
            args: &["--version"],
            contains: None,
        }),
        installer: Installer::Bundled,
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
        name: "vitest",
        check: None,
        installer: Installer::NodePackage {
            package: "vitest",
            version: None,
            dev: true,
        },
        for_signals: &[SignalKind::Test, SignalKind::Coverage],
        opt_in: false,
    },
    CatalogEntry {
        name: "@vitest/coverage-v8",
        check: None,
        installer: Installer::NodePackage {
            package: "@vitest/coverage-v8",
            version: None,
            dev: true,
        },
        for_signals: &[SignalKind::Coverage],
        opt_in: false,
    },
    CatalogEntry {
        name: "eslint",
        check: None,
        installer: Installer::NodePackage {
            package: "eslint",
            version: None,
            dev: true,
        },
        for_signals: &[SignalKind::Complexity],
        opt_in: false,
    },
    CatalogEntry {
        name: "@stylistic/eslint-plugin",
        check: None,
        installer: Installer::NodePackage {
            package: "@stylistic/eslint-plugin",
            version: None,
            dev: true,
        },
        for_signals: &[SignalKind::Complexity],
        opt_in: false,
    },
    CatalogEntry {
        name: "@stryker-mutator/core",
        check: None,
        installer: Installer::NodePackage {
            package: "@stryker-mutator/core",
            version: None,
            dev: true,
        },
        for_signals: &[SignalKind::Mutation],
        opt_in: true,
    },
];
