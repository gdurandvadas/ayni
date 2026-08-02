//! Python adapter implementing `LanguageAdapter` and `SignalCollector` from `ayni-core`.

pub mod adapter;
pub mod catalog;
pub mod collectors;
mod discovery;
mod package_manager;

pub use adapter::PythonAdapter;
