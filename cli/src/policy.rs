//! Application-side loading for policy snapshots.

use ayni_core::AyniPolicy;
use std::fs;
use std::path::Path;

pub(crate) fn load_from_path(config_path: &Path) -> Result<AyniPolicy, String> {
    let content = fs::read_to_string(config_path)
        .map_err(|error| format!("failed to read {}: {error}", config_path.display()))?;
    AyniPolicy::parse(&content)
        .map_err(|error| format!("failed to parse {}: {error}", config_path.display()))
}

#[cfg(test)]
mod tests {
    use super::load_from_path;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn reads_and_parses_policy_from_config_path() {
        let directory = TempDir::new().expect("directory");
        let config = directory.path().join("custom.toml");
        fs::write(&config, "[checks]\nsize = true\n").expect("policy");

        assert!(load_from_path(&config).expect("policy").checks.size);
    }
}
