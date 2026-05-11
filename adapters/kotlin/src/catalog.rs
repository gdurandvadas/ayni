use ayni_core::{CatalogEntry, Installer, SignalKind};

pub static KOTLIN_CATALOG: &[CatalogEntry] = &[
    CatalogEntry {
        name: "gradle-test",
        check: None,
        installer: Installer::GradleTask { task: "test" },
        for_signals: &[SignalKind::Test],
        opt_in: false,
    },
    CatalogEntry {
        name: "koverXmlReport",
        check: None,
        installer: Installer::GradleTask {
            task: "koverXmlReport",
        },
        for_signals: &[SignalKind::Coverage],
        opt_in: false,
    },
    CatalogEntry {
        name: "detekt",
        check: None,
        installer: Installer::GradleTask { task: "detekt" },
        for_signals: &[SignalKind::Complexity],
        opt_in: false,
    },
    CatalogEntry {
        name: "pitest",
        check: None,
        installer: Installer::GradleTask { task: "pitest" },
        for_signals: &[SignalKind::Mutation],
        opt_in: true,
    },
];
