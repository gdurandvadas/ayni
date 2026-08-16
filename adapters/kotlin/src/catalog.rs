use ayni_core::{CatalogEntry, SignalKind};

/// Tool catalog for the Kotlin adapter.
pub static KOTLIN_CATALOG: &[CatalogEntry] = &[
    CatalogEntry {
        name: "gradle-test",
        for_signals: &[SignalKind::Test],
        opt_in: false,
    },
    CatalogEntry {
        name: "kover",
        for_signals: &[SignalKind::Coverage],
        opt_in: false,
    },
    CatalogEntry {
        name: "jacoco",
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

#[cfg(test)]
mod tests {
    use super::KOTLIN_CATALOG;

    #[test]
    fn managed_collector_tool_names_are_cataloged() {
        for tool in ["kover", "jacoco", "detekt", "pitest"] {
            assert!(
                KOTLIN_CATALOG.iter().any(|entry| entry.name == tool),
                "{tool}"
            );
        }
    }
}
