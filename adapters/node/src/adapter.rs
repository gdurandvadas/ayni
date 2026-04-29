use crate::catalog::NODE_CATALOG;
use crate::collectors::NodeCollector;
use ayni_core::{
    CatalogEntry, DetectResult, Language, LanguageAdapter, LanguageProfile, SignalCollector,
    detect_node_package_manager,
};
use std::path::Path;

#[derive(Debug, Default)]
pub struct NodeAdapter {
    collector: NodeCollector,
}

impl NodeAdapter {
    #[must_use]
    pub fn new() -> Self {
        Self {
            collector: NodeCollector,
        }
    }
}

impl LanguageAdapter for NodeAdapter {
    fn language(&self) -> Language {
        Language::Node
    }

    fn detect(&self, root: &Path) -> DetectResult {
        let manifest = root.join("package.json");
        if !manifest.is_file() {
            return DetectResult {
                detected: false,
                confidence: 0,
                reason: Some(format!("package.json not found at {}", root.display())),
            };
        }

        let pm = detect_node_package_manager(root);
        let confidence = if pm.is_some() { 100 } else { 60 };
        let reason = if let Some(pm) = pm {
            format!(
                "package.json found at {}; package manager resolved as {}",
                root.display(),
                pm.executable()
            )
        } else {
            format!(
                "package.json found at {}; no lockfile/packageManager field (default runtime fallback)",
                root.display()
            )
        };

        DetectResult {
            detected: true,
            confidence,
            reason: Some(reason),
        }
    }

    fn profile(&self) -> LanguageProfile {
        LanguageProfile {
            language: Language::Node,
            default_file_globs: vec![
                String::from("*.js"),
                String::from("*.jsx"),
                String::from("*.ts"),
                String::from("*.tsx"),
                String::from("*.mjs"),
                String::from("*.cjs"),
            ],
        }
    }

    fn catalog(&self) -> &'static [CatalogEntry] {
        NODE_CATALOG
    }

    fn collector(&self) -> &dyn SignalCollector {
        &self.collector
    }
}
