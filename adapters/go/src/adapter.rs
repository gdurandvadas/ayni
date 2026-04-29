use crate::catalog::GO_CATALOG;
use crate::collectors::GoCollector;
use ayni_core::{
    CatalogEntry, DetectResult, Language, LanguageAdapter, LanguageProfile, SignalCollector,
};
use std::path::Path;

#[derive(Debug, Default)]
pub struct GoAdapter {
    collector: GoCollector,
}

impl GoAdapter {
    #[must_use]
    pub fn new() -> Self {
        Self {
            collector: GoCollector,
        }
    }
}

impl LanguageAdapter for GoAdapter {
    fn language(&self) -> Language {
        Language::Go
    }

    fn detect(&self, root: &Path) -> DetectResult {
        let detected = root.join("go.mod").is_file();
        DetectResult {
            detected,
            confidence: if detected { 100 } else { 0 },
            reason: if detected {
                Some(format!("go.mod found at {}", root.display()))
            } else {
                Some(format!("go.mod not found at {}", root.display()))
            },
        }
    }

    fn profile(&self) -> LanguageProfile {
        LanguageProfile {
            language: Language::Go,
            default_file_globs: vec![String::from("*.go")],
        }
    }

    fn catalog(&self) -> &'static [CatalogEntry] {
        GO_CATALOG
    }

    fn collector(&self) -> &dyn SignalCollector {
        &self.collector
    }
}
