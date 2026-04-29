use crate::adapter::LanguageAdapter;
use std::path::Path;
use std::sync::Arc;

#[derive(Default)]
pub struct AdapterRegistry {
    adapters: Vec<Arc<dyn LanguageAdapter>>,
}

impl AdapterRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self {
            adapters: Vec::new(),
        }
    }

    pub fn register(&mut self, adapter: Arc<dyn LanguageAdapter>) {
        self.adapters.push(adapter);
    }

    #[must_use]
    pub fn adapters(&self) -> &[Arc<dyn LanguageAdapter>] {
        &self.adapters
    }

    #[must_use]
    pub fn detect(&self, root: &Path) -> Vec<Arc<dyn LanguageAdapter>> {
        self.adapters
            .iter()
            .filter(|adapter| adapter.detect(root).detected)
            .cloned()
            .collect()
    }
}
