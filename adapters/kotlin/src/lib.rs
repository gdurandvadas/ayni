//! Kotlin adapter implementing Gradle-backed Ayni signals.

mod adapter;
pub mod catalog;
pub mod collectors;
mod discovery;
mod environment;
mod environment_resolution;
pub mod install;
mod preparation;

pub use adapter::KotlinAdapter;
