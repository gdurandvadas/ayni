use ayni_core::{CatalogEntry, Installer, SignalKind, VersionCheck};

/// Tool catalog for the Go adapter.
///
/// Drives `ayni install --language go` and keeps external tool requirements
/// centralized and signal-scoped.
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
            module: "github.com/fzipp/gocyclo/cmd/gocyclo",
            version: None,
        },
        for_signals: &[SignalKind::Complexity],
        opt_in: false,
    },
];
