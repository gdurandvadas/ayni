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

pub(crate) struct Reconciliation;
impl ayni_core::ToolingReconciliationCapability for Reconciliation {
    fn language(&self) -> ayni_core::Language {
        ayni_core::Language::Go
    }
    fn plan(
        &self,
        request: &ayni_core::ToolingRequest,
    ) -> Result<ayni_core::ToolingPlan, ayni_core::AdapterError> {
        let mut plan = ayni_adapters_common::tooling::baseline_plan(
            request,
            crate::catalog::GO_CATALOG,
            GO_TOOLS,
        )?;
        match crate::environment::tooling_owner(
            request.repo_root(),
            &request.repo_root().join(&request.target().root),
        ) {
            Ok(owner) => {
                plan.owner_root = ayni_adapters_common::repository::repository_relative(
                    request.repo_root(),
                    &owner,
                )
                .map_err(|cause| ayni_core::AdapterError::new(self.language(), cause))?
            }
            Err(cause) => plan
                .conflicts
                .push(ayni_adapters_common::tooling::diagnostic(
                    "tooling.native_metadata",
                    cause.to_string(),
                    None,
                )),
        }
        Ok(plan)
    }
}

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
