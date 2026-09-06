//! uv development-group baselines. Project ownership still consumes native locks.
use ayni_core::ManagedToolSpec;

pub static PYTHON_TOOLS: &[ManagedToolSpec] = &[
    ManagedToolSpec::runtime("python"),
    ManagedToolSpec::project("pytest", "9.0.3"),
    ManagedToolSpec::project("pytest-json-report", "1.5.0"),
    ManagedToolSpec::project("pytest-cov", "6.0.0"),
    ManagedToolSpec::project("coverage", "7.6.12"),
    ManagedToolSpec::project("complexipy", "7.0.1"),
    // 2.x supplies the junitxml command consumed by the mutation collector.
    ManagedToolSpec::project("mutmut", "2.5.1"),
];

pub(crate) struct Reconciliation;
impl ayni_core::ToolingReconciliationCapability for Reconciliation {
    fn language(&self) -> ayni_core::Language {
        ayni_core::Language::Python
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
        let adapter = crate::PythonAdapter::new();
        assert_eq!(adapter.managed_tool_specs(), PYTHON_TOOLS);
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
        let adapter = crate::PythonAdapter::new();
        let normal = BTreeSet::from([
            SignalKind::Test,
            SignalKind::Coverage,
            SignalKind::Complexity,
        ]);
        let selected = select_managed_tools(adapter.catalog(), PYTHON_TOOLS, &normal).unwrap();
        assert!(
            selected
                .iter()
                .all(|(spec, _)| spec.catalog_name != "mutmut")
        );
        let selected = select_managed_tools(
            adapter.catalog(),
            PYTHON_TOOLS,
            &BTreeSet::from([SignalKind::Mutation]),
        )
        .unwrap();
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].0.catalog_name, "mutmut");
    }

    #[test]
    fn baselines_have_native_declaration_fixtures_including_mutation() {
        let example: toml::Value =
            toml::from_str(include_str!("../../../examples/python/mono/pyproject.toml")).unwrap();
        let mutation: toml::Value = toml::from_str(include_str!(
            "../tests/fixtures/tooling/mutmut/pyproject.toml"
        ))
        .unwrap();
        let declarations = example["dependency-groups"]["dev"]
            .as_array()
            .unwrap()
            .iter()
            .chain(mutation["dependency-groups"]["dev"].as_array().unwrap())
            .filter_map(toml::Value::as_str)
            .collect::<BTreeSet<_>>();
        for spec in PYTHON_TOOLS
            .iter()
            .filter(|spec| spec.exact_version().is_some())
        {
            assert!(declarations.contains(
                format!("{}=={}", spec.catalog_name, spec.exact_version().unwrap()).as_str()
            ));
        }
    }
}
