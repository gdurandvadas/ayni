//! Node adapter implementing `LanguageAdapter` and `SignalCollector` from `ayni-core`.

mod adapter;
pub mod catalog;
pub mod collectors;
mod discovery;
mod environment;
mod environment_resolution;
mod impact;
mod package_manager;
mod preparation;

pub use adapter::NodeAdapter;
