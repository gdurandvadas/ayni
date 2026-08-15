//! Python adapter implementing `LanguageAdapter` and `SignalCollector` from `ayni-core`.

pub mod adapter;
pub mod catalog;
pub mod collectors;
mod discovery;
mod environment;
mod environment_resolution;
mod package_manager;
mod preparation;

pub use adapter::PythonAdapter;
