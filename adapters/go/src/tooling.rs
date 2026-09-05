//! Go signal tools never become application go.mod dependencies.
use ayni_core::{ManagedToolSpec, ToolBaseline, ToolIntegration};

pub const GOCYCLO_VERSION: &str = "0.6.0";
pub static GO_TOOLS: &[ManagedToolSpec] = &[
    ManagedToolSpec::runtime("go"),
    ManagedToolSpec {
        catalog_name: "gocyclo",
        baseline: ToolBaseline::Exact(GOCYCLO_VERSION),
        integration: ToolIntegration::Isolated {
            provider: "go:github.com/fzipp/gocyclo/cmd/gocyclo",
        },
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use ayni_core::{LanguageAdapter, SignalKind, select_managed_tools, validate_managed_tools};
    use std::collections::BTreeSet;

    #[test]
    fn inventory_covers_catalog_once_and_builtin_signals_need_no_external_tools() {
        let adapter = crate::GoAdapter::new();
        assert_eq!(adapter.managed_tool_specs(), GO_TOOLS);
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
}
