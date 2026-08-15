//! Kotlin exact environment resolution through mise.
use ayni_core::{
    AdapterError, EnvironmentResolutionCapability, EnvironmentResolutionRequest, Language,
    TargetEnvironment, VersionRequirement,
};
use std::time::Duration;

#[derive(Debug, Default)]
pub(crate) struct KotlinEnvironmentResolutionCapability;
impl EnvironmentResolutionCapability for KotlinEnvironmentResolutionCapability {
    fn language(&self) -> Language {
        Language::Kotlin
    }
    fn resolve(
        &self,
        request: &EnvironmentResolutionRequest,
    ) -> Result<TargetEnvironment, AdapterError> {
        let mut target = request.target().clone();
        for runtime in &mut target.runtimes {
            runtime.version = resolve_mise(request, &runtime.runtime, &runtime.version)?;
        }
        if let Some(manager) = &mut target.package_manager {
            manager.version = resolve_mise(request, &manager.family, &manager.version)?;
        }
        Ok(target)
    }
}
fn resolve_mise(
    request: &EnvironmentResolutionRequest,
    name: &str,
    requirement: &VersionRequirement,
) -> Result<VersionRequirement, AdapterError> {
    if requirement.is_exact() {
        return Ok(requirement.clone());
    }
    let selector = match requirement {
        VersionRequirement::Selector { expression }
        | VersionRequirement::Compatibility { expression } => expression,
        VersionRequirement::Minimum { version } => version,
        VersionRequirement::Unresolved { reason } => {
            return Err(error(format!("cannot resolve {name}: {reason}")));
        }
        VersionRequirement::Exact { .. } => unreachable!(),
    };
    let query = format!("{name}@{selector}");
    let output = ayni_adapters_common::exec::run_command(
        request.repo_root(),
        "mise",
        &[
            "--no-config".into(),
            "--no-env".into(),
            "--no-hooks".into(),
            "latest".into(),
            query.clone(),
        ],
        Duration::from_secs(120),
    )
    .map_err(|cause| {
        AdapterError::execution(
            Language::Kotlin,
            format!("failed to run mise for {name}: {cause}"),
        )
    })?;
    if !output.status.success() {
        return Err(AdapterError::execution(
            Language::Kotlin,
            format!("mise could not resolve {query}"),
        ));
    }
    let value = String::from_utf8(output.stdout).map_err(|cause| {
        AdapterError::execution(
            Language::Kotlin,
            format!("mise returned non-UTF-8 output for {name}: {cause}"),
        )
    })?;
    let value = value.trim();
    if !is_exact_provider_version(value) {
        return Err(error(format!(
            "mise did not return an exact version for {query}: {value}"
        )));
    }
    if name == "java" && !java_version_matches(value, selector) {
        return Err(error(format!(
            "mise resolved {query} to incompatible Java version {value}"
        )));
    }
    VersionRequirement::exact(value).map_err(error)
}
fn is_exact_provider_version(value: &str) -> bool {
    !value.is_empty()
        && !matches!(value, "latest" | "stable" | "current")
        && value.bytes().any(|byte| byte.is_ascii_digit())
        && (value.contains('.') || value.contains('-'))
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_' | b'+'))
}

fn java_version_matches(value: &str, selector: &str) -> bool {
    let selector = selector
        .split(['.', '-', '+'])
        .find(|part| part.bytes().all(|byte| byte.is_ascii_digit()))
        .unwrap_or(selector);
    if !selector.bytes().all(|byte| byte.is_ascii_digit()) {
        return true;
    }
    value == selector
        || value.starts_with(&format!("{selector}."))
        || value.contains(&format!("-{selector}."))
        || value.contains(&format!("-{selector}+"))
}

fn error(message: impl std::fmt::Display) -> AdapterError {
    AdapterError::new(Language::Kotlin, message.to_string())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accepts_only_exact_mise_versions() {
        assert!(is_exact_provider_version("21.0.6"));
        assert!(is_exact_provider_version("8.10.2"));
        assert!(!is_exact_provider_version("21"));
        assert!(!is_exact_provider_version("latest"));
        assert!(is_exact_provider_version("8.10"));
        assert!(is_exact_provider_version("temurin-21.0.6+7.0.LTS"));
        assert!(java_version_matches("temurin-21.0.6+7.0.LTS", "21"));
        assert!(!java_version_matches("temurin-17.0.14+7", "21"));
    }
}
