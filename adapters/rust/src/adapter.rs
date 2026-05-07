use crate::catalog::RUST_CATALOG;
use crate::collectors::RustCollector;
use crate::discovery;
use ayni_core::{
    CatalogEntry, DetectResult, Language, LanguageAdapter, LanguageProfile, SignalCollector,
};
use std::path::Path;

#[derive(Debug, Default)]
pub struct RustAdapter {
    collector: RustCollector,
}

impl RustAdapter {
    #[must_use]
    pub fn new() -> Self {
        Self {
            collector: RustCollector,
        }
    }
}

impl LanguageAdapter for RustAdapter {
    fn language(&self) -> Language {
        Language::Rust
    }

    fn detect(&self, root: &Path) -> DetectResult {
        let detected = root.join("Cargo.toml").is_file();
        DetectResult {
            detected,
            confidence: if detected { 100 } else { 0 },
            reason: if detected {
                Some(format!("Cargo.toml found at {}", root.display()))
            } else {
                Some(format!("Cargo.toml not found at {}", root.display()))
            },
        }
    }

    fn discover_roots(&self, repo_root: &Path) -> Vec<String> {
        discovery::discover_roots(repo_root)
    }

    fn profile(&self) -> LanguageProfile {
        LanguageProfile {
            language: Language::Rust,
            default_file_globs: vec![String::from("*.rs")],
        }
    }

    fn catalog(&self) -> &'static [CatalogEntry] {
        RUST_CATALOG
    }

    fn collector(&self) -> &dyn SignalCollector {
        &self.collector
    }
}
