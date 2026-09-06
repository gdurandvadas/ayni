//! Native project dependency baselines, exercised by examples/node/mono.
use ayni_core::ManagedToolSpec;

pub static NODE_TOOLS: &[ManagedToolSpec] = &[
    ManagedToolSpec::runtime("node"),
    ManagedToolSpec::project("vitest", "3.2.7"),
    ManagedToolSpec::project("@vitest/coverage-v8", "3.2.7"),
    ManagedToolSpec::project("eslint", "9.39.5"),
    ManagedToolSpec::project("@typescript-eslint/parser", "8.67.0"),
];

pub(crate) struct Reconciliation;
impl ayni_core::ToolingReconciliationCapability for Reconciliation {
    fn language(&self) -> ayni_core::Language {
        ayni_core::Language::Node
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
        let adapter = crate::NodeAdapter::new();
        assert_eq!(adapter.managed_tool_specs(), NODE_TOOLS);
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
    fn baseline_versions_match_native_example_declarations() {
        let manifest: serde_json::Value =
            serde_json::from_str(include_str!("../../../examples/node/mono/package.json")).unwrap();
        for spec in NODE_TOOLS
            .iter()
            .filter(|spec| spec.exact_version().is_some())
        {
            assert_eq!(
                manifest["devDependencies"][spec.catalog_name].as_str(),
                spec.exact_version()
            );
        }
    }
}
