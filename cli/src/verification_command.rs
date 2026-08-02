//! Materialization of adapter-owned finding targets into public CLI commands.

use crate::signal_kind_slug;
use ayni_core::{
    AdapterRegistry, FindingError, Language, RunArtifact, SignalKind, VerificationTarget,
};

/// Materialize adapter-owned targets only after capability validation and before
/// any terminal, Markdown, JSON, or persisted artifact presentation.
pub(crate) fn materialize_finding_commands(
    artifact: &mut RunArtifact,
    registry: &AdapterRegistry,
) -> Result<(), String> {
    let mut findings = Vec::with_capacity(artifact.rows.len());
    for row in &artifact.rows {
        let adapter = registry
            .adapters()
            .iter()
            .find(|adapter| adapter.language() == row.language)
            .ok_or_else(|| format!("{} adapter unavailable", row.language))?;
        let mut row_findings = adapter
            .findings_for(row, &row.scope.workspace_root)
            .map_err(|error| format!("failed to map {:?} findings: {error}", row.kind))?;
        let configured_root = row.scope.path.as_deref().unwrap_or(".");
        row_findings
            .render_commands(|target| {
                adapter
                    .verification_selector_support(row.kind)
                    .validate_target(row.kind, target)?;
                Ok(render_verification_command(
                    &artifact.metadata.config_path,
                    row.kind,
                    row.language,
                    configured_root,
                    target,
                ))
            })
            .map_err(|error: FindingError| error.to_string())?;
        findings.push(row_findings);
    }
    artifact.findings = findings;
    Ok(())
}

fn render_verification_command(
    config_path: &str,
    kind: SignalKind,
    language: Language,
    configured_root: &str,
    target: &VerificationTarget,
) -> String {
    let mut command = format!(
        "ayni verify {} --config {} --language {} --root {}",
        signal_kind_slug(kind),
        shell_quote(config_path),
        language.as_str(),
        shell_quote(configured_root),
    );
    if let Some(file) = &target.file {
        command.push_str(&format!(" --file {}", shell_quote(file)));
    }
    if let Some(package) = &target.package {
        command.push_str(&format!(" --package {}", shell_quote(package)));
    }
    if let Some(name) = &target.name {
        command.push_str(&format!(" --name {}", shell_quote(name)));
    }
    command
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

#[cfg(test)]
mod tests {
    use super::{render_verification_command, shell_quote};
    use ayni_core::{Language, SignalKind, VerificationTarget};

    #[test]
    fn verification_command_is_exact_and_shell_safe() {
        assert_eq!(
            render_verification_command(
                "policies/it's hostile $(nope).toml",
                SignalKind::Test,
                Language::Node,
                "apps/a weird;root",
                &VerificationTarget {
                    file: Some(String::from("tests/a weird;name.test.js")),
                    package: None,
                    name: Some(String::from("it's focused $(nope)")),
                },
            ),
            "ayni verify test --config 'policies/it'\"'\"'s hostile $(nope).toml' --language node --root 'apps/a weird;root' --file 'tests/a weird;name.test.js' --name 'it'\"'\"'s focused $(nope)'"
        );
    }

    #[test]
    fn repository_root_is_rendered_as_normalized_dot() {
        assert_eq!(
            render_verification_command(
                "./.ayni.toml",
                SignalKind::Coverage,
                Language::Rust,
                ".",
                &VerificationTarget::default(),
            ),
            "ayni verify coverage --config './.ayni.toml' --language rust --root '.'"
        );
    }

    #[test]
    fn shell_quote_always_quotes_empty_and_hostile_values() {
        assert_eq!(shell_quote(""), "''");
        assert_eq!(shell_quote("a' b"), "'a'\"'\"' b'");
    }
}
