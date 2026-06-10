//! Shared infrastructure for Ayni language adapters.
//!
//! This crate owns the cross-language plumbing every adapter needs: command
//! execution with timeouts, repository path normalization, failure
//! classification scaffolding, lightweight XML attribute parsing, and
//! marker-file root discovery. Language-specific behavior (which tools to
//! run, how to parse their reports) stays in the per-language adapter crates.

pub mod catalog;
pub mod discovery;
pub mod exec;
pub mod failure;
pub mod paths;
pub mod reports;
pub mod xml;
