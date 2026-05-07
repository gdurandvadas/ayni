use ayni_core::{CatalogEntry, Installer, SignalKind};

/// Drives `ayni install --language python`.
pub static PYTHON_CATALOG: &[CatalogEntry] = &[
    CatalogEntry {
        name: "python",
        check: None,
        installer: Installer::PythonRuntime,
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
        check: None,
        installer: Installer::PythonPackage {
            package: "pytest",
            import_name: "pytest",
            version: None,
            dev: true,
        },
        for_signals: &[SignalKind::Test, SignalKind::Coverage],
        opt_in: false,
    },
    CatalogEntry {
        name: "pytest-json-report",
        check: None,
        installer: Installer::PythonPackage {
            package: "pytest-json-report",
            import_name: "pytest_jsonreport",
            version: None,
            dev: true,
        },
        for_signals: &[SignalKind::Test],
        opt_in: false,
    },
    CatalogEntry {
        name: "pytest-cov",
        check: None,
        installer: Installer::PythonPackage {
            package: "pytest-cov",
            import_name: "pytest_cov",
            version: None,
            dev: true,
        },
        for_signals: &[SignalKind::Coverage],
        opt_in: false,
    },
    CatalogEntry {
        name: "coverage",
        check: None,
        installer: Installer::PythonPackage {
            package: "coverage",
            import_name: "coverage",
            version: None,
            dev: true,
        },
        for_signals: &[SignalKind::Coverage],
        opt_in: false,
    },
    CatalogEntry {
        name: "complexipy",
        check: None,
        installer: Installer::UvTool {
            package: "complexipy",
            version: None,
        },
        for_signals: &[SignalKind::Complexity],
        opt_in: false,
    },
    CatalogEntry {
        name: "mutmut",
        check: None,
        installer: Installer::PythonPackage {
            package: "mutmut",
            import_name: "mutmut",
            version: None,
            dev: true,
        },
        for_signals: &[SignalKind::Mutation],
        opt_in: true,
    },
];
