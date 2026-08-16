//! Declarative tool catalog metadata.
//!
//! Adapters use catalog entries to identify the tools each signal depends on.
//! Environment planning and quality collection own provisioning and execution;
//! catalogs never inspect, install, or mutate a checkout.

use crate::signal::SignalKind;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogEntry {
    pub name: &'static str,
    pub for_signals: &'static [SignalKind],
    pub opt_in: bool,
}
