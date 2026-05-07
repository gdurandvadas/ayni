use ayni_core::{AdapterRegistry, Language};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

pub fn discover_language_roots(
    repo_root: &Path,
    enabled_languages: &[Language],
    language_filter: Option<Language>,
    registry: &AdapterRegistry,
) -> BTreeMap<Language, Vec<String>> {
    let enabled_set: BTreeSet<Language> = enabled_languages.iter().copied().collect();
    let mut discovered = BTreeMap::new();
    for adapter in registry.adapters() {
        let language = adapter.language();
        if let Some(filter) = language_filter
            && filter != language
        {
            continue;
        }
        if !enabled_set.contains(&language) {
            continue;
        }
        let mut roots = adapter.discover_roots(repo_root);
        roots.sort();
        roots.dedup();
        discovered.insert(
            language,
            if roots.is_empty() {
                vec![String::from(".")]
            } else {
                roots
            },
        );
    }
    discovered
}
