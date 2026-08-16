use ayni_adapters_go::GoAdapter;
use ayni_adapters_kotlin::KotlinAdapter;
use ayni_adapters_node::NodeAdapter;
use ayni_adapters_python::PythonAdapter;
use ayni_adapters_rust::RustAdapter;
use ayni_core::AdapterRegistry;
use std::sync::Arc;

pub(crate) fn build_registry() -> AdapterRegistry {
    let mut registry = AdapterRegistry::new();
    registry.register(Arc::new(GoAdapter::new()));
    registry.register(Arc::new(RustAdapter::new()));
    registry.register(Arc::new(NodeAdapter::new()));
    registry.register(Arc::new(PythonAdapter::new()));
    registry.register(Arc::new(KotlinAdapter::new()));
    registry
}
