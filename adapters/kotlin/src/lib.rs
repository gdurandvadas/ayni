//! Kotlin adapter implementing Gradle-backed Ayni signals.

mod adapter;
pub mod catalog;
pub mod collectors;
mod discovery;
pub mod install;

pub use adapter::KotlinAdapter;
