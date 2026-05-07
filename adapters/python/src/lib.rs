//! Python adapter implementing `LanguageAdapter` and `SignalCollector` from `ayni-core`.

pub mod adapter;
pub mod catalog;
pub mod collectors;
mod discovery;

pub use adapter::PythonAdapter;
