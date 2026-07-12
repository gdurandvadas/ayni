use ayni_core::{AdapterRegistry, Language};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

pub fn discover_language_roots(
    repo_root: &Path,
    enabled_languages: &[Language],
    selected_languages: &BTreeSet<Language>,
    registry: &AdapterRegistry,
) -> BTreeMap<Language, Vec<String>> {
    let enabled_set: BTreeSet<Language> = enabled_languages.iter().copied().collect();
    let mut discovered = BTreeMap::new();
    for adapter in registry.adapters() {
        let language = adapter.language();
        if !selected_languages.is_empty() && !selected_languages.contains(&language) {
            continue;
        }
        if !enabled_set.contains(&language) {
            continue;
        }
        discovered.insert(
            language,
            adapter.discover_project_roots(repo_root).policy_roots(),
        );
    }
    discovered
}
