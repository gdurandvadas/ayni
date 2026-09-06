//! Canonical isolated tools and toolchain components for Rust.
use ayni_core::{ManagedToolSpec, ToolBaseline, ToolIntegration};

pub const CARGO_LLVM_COV_VERSION: &str = "0.8.5";
pub const RUST_CODE_ANALYSIS_VERSION: &str = "0.0.25";

pub static RUST_TOOLS: &[ManagedToolSpec] = &[
    ManagedToolSpec::runtime("cargo"),
    ManagedToolSpec {
        catalog_name: "llvm-tools-preview",
        baseline: ToolBaseline::Toolchain,
        integration: ToolIntegration::ToolchainComponent,
    },
    ManagedToolSpec {
        catalog_name: "cargo-llvm-cov",
        baseline: ToolBaseline::Exact(CARGO_LLVM_COV_VERSION),
        integration: ToolIntegration::Isolated {
            provider: "cargo-install",
        },
    },
    ManagedToolSpec {
        catalog_name: "rust-code-analysis-cli",
        baseline: ToolBaseline::Exact(RUST_CODE_ANALYSIS_VERSION),
        integration: ToolIntegration::Isolated {
            provider: "cargo-install",
        },
    },
];

pub(crate) struct Reconciliation;
impl ayni_core::ToolingReconciliationCapability for Reconciliation {
    fn language(&self) -> ayni_core::Language {
        ayni_core::Language::Rust
    }
    fn plan(
        &self,
        request: &ayni_core::ToolingRequest,
    ) -> Result<ayni_core::ToolingPlan, ayni_core::AdapterError> {
        let mut plan = ayni_adapters_common::tooling::baseline_plan(
            request,
            crate::catalog::RUST_CATALOG,
            RUST_TOOLS,
        )?;
        match crate::environment::workspace_root(
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
        let adapter = crate::RustAdapter::new();
        assert_eq!(adapter.managed_tool_specs(), RUST_TOOLS);
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
