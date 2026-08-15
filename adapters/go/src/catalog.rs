use ayni_core::{CatalogEntry, Installer, SignalKind, VersionCheck};

/// Module and exact version used by both host installation and managed provisioning.
pub(crate) const GOCYCLO_MODULE: &str = "github.com/fzipp/gocyclo/cmd/gocyclo";
pub(crate) const GOCYCLO_VERSION: &str = "0.6.0";

/// Tool catalog for the Go adapter.
///
/// Drives host diagnostics and keeps external tool requirements centralized
/// and signal-scoped.
pub static GO_CATALOG: &[CatalogEntry] = &[
    CatalogEntry {
        name: "go",
        check: Some(VersionCheck {
            command: "go",
            args: &["version"],
            contains: None,
        }),
        installer: Installer::Bundled,
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
        check: Some(VersionCheck {
            command: "gocyclo",
            // `gocyclo -h` exits non-zero on some versions; use a real directory input probe.
            args: &["."],
            contains: None,
        }),
        installer: Installer::GoInstall {
            module: GOCYCLO_MODULE,
            version: Some(GOCYCLO_VERSION),
        },
        for_signals: &[SignalKind::Complexity],
        opt_in: false,
    },
];
