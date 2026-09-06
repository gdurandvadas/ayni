//! Gradle plugin baselines and coverage-provider selection.
use ayni_core::{ManagedToolSpec, ToolBaseline, ToolIntegration};

pub const KOVER: ManagedToolSpec = ManagedToolSpec {
    catalog_name: "kover",
    baseline: ToolBaseline::Exact("0.9.8"),
    integration: ToolIntegration::GradlePlugin {
        plugin_ids: &[
            "org.jetbrains.kotlinx.kover",
            "org.jetbrains.kotlinx.kover.gradle.plugin",
        ],
    },
};
pub const JACOCO: ManagedToolSpec = ManagedToolSpec {
    catalog_name: "jacoco",
    baseline: ToolBaseline::Exact("0.8.12"),
    integration: ToolIntegration::GradleToolVersion {
        plugin_id: "jacoco",
    },
};
pub const DETEKT: ManagedToolSpec = ManagedToolSpec {
    catalog_name: "detekt",
    baseline: ToolBaseline::Exact("1.23.8"),
    integration: ToolIntegration::GradlePlugin {
        plugin_ids: &["io.gitlab.arturbosch.detekt"],
    },
};
pub const PITEST: ManagedToolSpec = ManagedToolSpec {
    catalog_name: "pitest",
    baseline: ToolBaseline::Exact("1.19.0"),
    integration: ToolIntegration::GradlePlugin {
        plugin_ids: &["info.solidsoft.pitest"],
    },
};

pub static KOTLIN_TOOLS: &[ManagedToolSpec] = &[
    ManagedToolSpec::runtime("gradle-test"),
    KOVER,
    JACOCO,
    DETEKT,
    PITEST,
];

/// Preserve an existing supported provider; Kover is preferred when absent.
/// Callers must diagnose conflicting declarations before choosing a provider.
pub fn coverage_baseline(existing: Option<&str>) -> Result<&'static ManagedToolSpec, String> {
    match existing {
        None | Some("kover") => Ok(&KOVER),
        Some("jacoco") => Ok(&JACOCO),
        Some(other) => Err(format!("unsupported Kotlin coverage provider: {other}")),
    }
}

pub(crate) struct Reconciliation;
impl ayni_core::ToolingReconciliationCapability for Reconciliation {
    fn language(&self) -> ayni_core::Language {
        ayni_core::Language::Kotlin
    }
    fn plan(
        &self,
        request: &ayni_core::ToolingRequest,
    ) -> Result<ayni_core::ToolingPlan, ayni_core::AdapterError> {
        crate::tooling_reconcile::plan(request)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ayni_core::{LanguageAdapter, SignalKind, select_managed_tools, validate_managed_tools};
    use std::collections::BTreeSet;

    #[test]
    fn inventory_covers_catalog_once_and_builtin_signals_need_no_external_tools() {
        let adapter = crate::KotlinAdapter::new();
        assert_eq!(adapter.managed_tool_specs(), KOTLIN_TOOLS);
        validate_managed_tools(adapter.catalog(), adapter.managed_tool_specs()).unwrap();
        assert!(
            select_managed_tools(
                adapter.catalog(),
                adapter.managed_tool_specs(),
                &BTreeSet::from([SignalKind::Size, SignalKind::Deps])
            )
            .unwrap()
            .is_empty()
        );
        assert!(
            select_managed_tools(
                adapter.catalog(),
                adapter.managed_tool_specs(),
                &BTreeSet::new()
            )
            .unwrap()
            .is_empty()
        );
    }

    #[test]
    fn mutation_is_selected_only_when_its_default_signal_is_enabled() {
        let adapter = crate::KotlinAdapter::new();
        let normal = BTreeSet::from([
            SignalKind::Test,
            SignalKind::Coverage,
            SignalKind::Complexity,
        ]);
        let selected = select_managed_tools(adapter.catalog(), KOTLIN_TOOLS, &normal).unwrap();
        assert!(
            selected
                .iter()
                .all(|(spec, _)| spec.catalog_name != "pitest")
        );
        let selected = select_managed_tools(
            adapter.catalog(),
            KOTLIN_TOOLS,
            &BTreeSet::from([SignalKind::Mutation]),
        )
        .unwrap();
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].0.catalog_name, "pitest");
    }

    #[test]
    fn coverage_preserves_supported_choices_and_prefers_kover_when_absent() {
        assert_eq!(coverage_baseline(None).unwrap(), &KOVER);
        assert_eq!(coverage_baseline(Some("jacoco")).unwrap(), &JACOCO);
        assert_eq!(coverage_baseline(Some("kover")).unwrap(), &KOVER);
        assert!(coverage_baseline(Some("unknown")).is_err());
    }
    #[test]
    fn every_plugin_baseline_has_a_native_fixture() {
        let fixtures = [
            include_str!("../../../examples/kotlin/mono/build.gradle.kts"),
            include_str!("../tests/fixtures/tooling/gradle/build.gradle.kts"),
        ];
        for spec in KOTLIN_TOOLS
            .iter()
            .filter(|spec| spec.exact_version().is_some())
        {
            let version = spec.exact_version().unwrap();
            let declaration = match spec.integration {
                ToolIntegration::GradlePlugin { plugin_ids } => {
                    format!("id(\"{}\") version \"{version}\"", plugin_ids[0])
                }
                ToolIntegration::GradleToolVersion { .. } => format!("toolVersion = \"{version}\""),
                _ => panic!("unexpected Kotlin integration"),
            };
            assert!(
                fixtures.iter().any(|text| text.contains(&declaration)),
                "{}",
                spec.catalog_name
            );
        }
    }
}
