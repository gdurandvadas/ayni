use crate::catalog::RUST_CATALOG;
use crate::collectors::RustCollector;
use crate::discovery;
use ayni_core::{
    CatalogEntry, DetectResult, ExecutionResolution, Language, LanguageAdapter, LanguageProfile,
    SignalCollector,
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

    fn resolve_execution(&self, repo_root: &Path, root: &Path) -> Option<ExecutionResolution> {
        if !root.join("Cargo.toml").is_file() {
            return None;
        }
        if let Some(workspace) = find_cargo_workspace_ancestor(repo_root, root) {
            return Some(ExecutionResolution {
                runner: String::from("cargo"),
                resolved_from: workspace.clone(),
                kind: String::from("workspace_ancestor"),
                source: String::from("Cargo workspace"),
                confidence: 90,
                ambiguous: false,
                install_cwd: workspace.clone(),
                exec_cwd: workspace,
            });
        }
        Some(ExecutionResolution::direct(
            "cargo",
            root.to_path_buf(),
            "Cargo.toml",
            100,
        ))
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

fn find_cargo_workspace_ancestor(repo_root: &Path, root: &Path) -> Option<std::path::PathBuf> {
    let mut current = root.parent();
    while let Some(path) = current {
        if !path.starts_with(repo_root) {
            break;
        }
        let manifest = path.join("Cargo.toml");
        if manifest.is_file() && cargo_manifest_has_workspace(&manifest) {
            return Some(path.to_path_buf());
        }
        current = path.parent();
    }
    None
}

fn cargo_manifest_has_workspace(path: &Path) -> bool {
    std::fs::read_to_string(path)
        .ok()
        .is_some_and(|content| content.lines().any(|line| line.trim() == "[workspace]"))
}

#[cfg(test)]
mod tests {
    use super::RustAdapter;
    use ayni_core::LanguageAdapter;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn resolves_cargo_workspace_ancestor() {
        let dir = TempDir::new().expect("tempdir");
        fs::write(
            dir.path().join("Cargo.toml"),
            r#"[workspace]
members = ["crates/api"]
"#,
        )
        .expect("workspace manifest");
        fs::create_dir_all(dir.path().join("crates/api")).expect("api dir");
        fs::write(
            dir.path().join("crates/api/Cargo.toml"),
            r#"[package]
name = "api"
version = "0.1.0"
edition = "2021"
"#,
        )
        .expect("api manifest");

        let adapter = RustAdapter::new();
        let resolution = adapter
            .resolve_execution(dir.path(), &dir.path().join("crates/api"))
            .expect("resolution");

        assert_eq!(resolution.runner, "cargo");
        assert_eq!(resolution.kind, "workspace_ancestor");
        assert_eq!(resolution.exec_cwd, dir.path());
    }
}
