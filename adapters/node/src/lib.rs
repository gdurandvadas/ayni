//! Node adapter implementing `LanguageAdapter` and `SignalCollector` from `ayni-core`.

mod adapter;
pub mod catalog;
pub mod collectors;
mod discovery;
mod environment;
mod package_manager;

pub use adapter::NodeAdapter;
