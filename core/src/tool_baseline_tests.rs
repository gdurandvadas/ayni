use super::*;

const CATALOG: &[CatalogEntry] = &[
    CatalogEntry {
        name: "runtime",
        for_signals: &[SignalKind::Size],
        opt_in: false,
    },
    CatalogEntry {
        name: "test",
        for_signals: &[SignalKind::Test, SignalKind::Coverage],
        opt_in: false,
    },
    CatalogEntry {
        name: "mutation",
        for_signals: &[SignalKind::Mutation],
        opt_in: true,
    },
];
const TOOLS: &[ManagedToolSpec] = &[
    ManagedToolSpec::runtime("runtime"),
    ManagedToolSpec::project("test", "1.2.3"),
    ManagedToolSpec::project("mutation", "2.0.0"),
];

#[test]
fn inventories_reject_missing_duplicate_and_uncataloged_tools() {
    assert!(validate_managed_tools(CATALOG, TOOLS).is_ok());
    assert!(validate_managed_tools(CATALOG, &TOOLS[..2]).is_err());
    let mut duplicate = TOOLS.to_vec();
    duplicate.push(TOOLS[0]);
    assert!(validate_managed_tools(CATALOG, &duplicate).is_err());
    duplicate = TOOLS.to_vec();
    duplicate[0].catalog_name = "unknown";
    assert!(validate_managed_tools(CATALOG, &duplicate).is_err());
    let mut catalog = CATALOG.to_vec();
    catalog.push(CATALOG[0].clone());
    assert!(validate_managed_tools(&catalog, TOOLS).is_err());
}

#[test]
fn only_default_signals_create_requirements_and_opt_in_is_not_implicit() {
    let selected = select_managed_tools(
        CATALOG,
        TOOLS,
        &BTreeSet::from([SignalKind::Coverage, SignalKind::Size]),
    )
    .unwrap();
    assert_eq!(selected, vec![(&TOOLS[1], vec![SignalKind::Coverage])]);
    // A custom command suppresses the corresponding default signal before selection.
    assert!(
        select_managed_tools(CATALOG, TOOLS, &BTreeSet::new())
            .unwrap()
            .is_empty()
    );
    let mutation =
        select_managed_tools(CATALOG, TOOLS, &BTreeSet::from([SignalKind::Mutation])).unwrap();
    assert_eq!(mutation, vec![(&TOOLS[2], vec![SignalKind::Mutation])]);
}

#[test]
fn baseline_and_integration_combinations_fail_closed() {
    for (baseline, integration) in [
        (
            ToolBaseline::Toolchain,
            ToolIntegration::ProjectDependency { package: "test" },
        ),
        (ToolBaseline::Exact("1.2.3"), ToolIntegration::Runtime),
        (
            ToolBaseline::Exact("latest"),
            ToolIntegration::Isolated {
                provider: "cargo-install",
            },
        ),
        (
            ToolBaseline::Exact("1.2.3"),
            ToolIntegration::Isolated { provider: "" },
        ),
        (
            ToolBaseline::Exact("1.2.3"),
            ToolIntegration::GradlePlugin { plugin_ids: &[] },
        ),
        (
            ToolBaseline::Exact("1.2.3"),
            ToolIntegration::GradlePlugin {
                plugin_ids: &["plugin", "plugin"],
            },
        ),
        (
            ToolBaseline::Exact("1.2.3"),
            ToolIntegration::GradleToolVersion {
                plugin_id: "bad\nplugin",
            },
        ),
    ] {
        let mut tools = TOOLS.to_vec();
        tools[1].baseline = baseline;
        tools[1].integration = integration;
        assert!(
            validate_managed_tools(CATALOG, &tools).is_err(),
            "{baseline:?} {integration:?}"
        );
    }
    let component = ManagedToolSpec {
        catalog_name: "runtime",
        baseline: ToolBaseline::Toolchain,
        integration: ToolIntegration::ToolchainComponent,
    };
    assert!(validate_managed_tools(&CATALOG[..1], &[component]).is_ok());
    assert_eq!(component.exact_version(), None);
    assert!(component.plugin_ids().is_empty());
}
