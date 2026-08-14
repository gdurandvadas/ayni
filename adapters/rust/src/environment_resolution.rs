use ayni_core::{
    AdapterError, EnvironmentResolutionCapability, EnvironmentResolutionRequest, Language,
    TargetEnvironment, VersionRequirement,
};
use std::time::Duration;

#[derive(Debug, Default)]
pub(crate) struct RustEnvironmentResolutionCapability;

impl EnvironmentResolutionCapability for RustEnvironmentResolutionCapability {
    fn language(&self) -> Language {
        Language::Rust
    }

    fn resolve(
        &self,
        request: &EnvironmentResolutionRequest,
    ) -> Result<TargetEnvironment, AdapterError> {
        let mut target = request.target().clone();
        for runtime in &mut target.runtimes {
            runtime.version = resolve_runtime(request, &runtime.version)?;
        }
        for tool in &mut target.signal_tools {
            tool.version = resolve_cargo_tool(request, &tool.tool, &tool.version)?;
        }
        Ok(target)
    }
}

fn resolve_runtime(
    request: &EnvironmentResolutionRequest,
    requirement: &VersionRequirement,
) -> Result<VersionRequirement, AdapterError> {
    if let VersionRequirement::Exact { .. } = requirement {
        return Ok(requirement.clone());
    }
    let selector = match requirement {
        VersionRequirement::Selector { expression }
        | VersionRequirement::Compatibility { expression } => expression.as_str(),
        VersionRequirement::Minimum { version } => version.as_str(),
        VersionRequirement::Unresolved { reason } => {
            return Err(error(format!("cannot resolve rust: {reason}")));
        }
        VersionRequirement::Exact { .. } => unreachable!(),
    };
    resolve_mise_query(
        request,
        "rust",
        &format!("rust@{selector}"),
        is_exact_rust_version,
    )
}

fn resolve_cargo_tool(
    request: &EnvironmentResolutionRequest,
    name: &str,
    requirement: &VersionRequirement,
) -> Result<VersionRequirement, AdapterError> {
    if let VersionRequirement::Exact { .. } = requirement {
        return Ok(requirement.clone());
    }
    let query = match requirement {
        VersionRequirement::Selector { expression }
        | VersionRequirement::Compatibility { expression } => format!("cargo:{name}@{expression}"),
        VersionRequirement::Minimum { version } => format!("cargo:{name}@{version}"),
        VersionRequirement::Unresolved { .. } => format!("cargo:{name}"),
        VersionRequirement::Exact { .. } => unreachable!(),
    };
    resolve_mise_query(request, name, &query, is_exact_cargo_version)
}

fn resolve_mise_query(
    request: &EnvironmentResolutionRequest,
    name: &str,
    query: &str,
    is_exact: fn(&str) -> bool,
) -> Result<VersionRequirement, AdapterError> {
    let args = vec![
        "--no-config".to_owned(),
        "--no-env".to_owned(),
        "--no-hooks".to_owned(),
        "latest".to_owned(),
        query.to_owned(),
    ];
    let output = ayni_adapters_common::exec::run_command(
        request.repo_root(),
        "mise",
        &args,
        Duration::from_secs(120),
    )
    .map_err(|cause| provider_error(format!("failed to run mise for {name}: {cause}")))?;
    if !output.status.success() {
        return Err(provider_error(format!("mise could not resolve {query}")));
    }
    let version = String::from_utf8(output.stdout).map_err(|cause| {
        provider_error(format!(
            "mise returned non-UTF-8 output for {name}: {cause}"
        ))
    })?;
    let version = version.trim();
    if !is_exact(version) {
        return Err(error(format!(
            "mise did not return an exact version for {query}: {version}"
        )));
    }
    VersionRequirement::exact(version).map_err(|cause| error(cause.to_string()))
}

fn is_exact_rust_version(value: &str) -> bool {
    let numeric = value.split('.').count() == 3
        && value
            .split('.')
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()));
    let dated = value.strip_prefix("nightly-").or_else(|| value.strip_prefix("beta-")).is_some_and(|date| {
        let parts = date.split('-').collect::<Vec<_>>();
        matches!(parts.as_slice(), [year, month, day] if year.len() == 4 && month.len() == 2 && day.len() == 2 && parts.iter().all(|part| part.bytes().all(|byte| byte.is_ascii_digit())))
    });
    numeric || dated
}

fn is_exact_cargo_version(value: &str) -> bool {
    semver::Version::parse(value).is_ok()
}

fn provider_error(message: impl Into<String>) -> AdapterError {
    AdapterError::execution(Language::Rust, message)
}

fn error(message: impl Into<String>) -> AdapterError {
    AdapterError::new(Language::Rust, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_floating_rust_channels_as_exact_versions() {
        assert!(!is_exact_rust_version("stable"));
        assert!(!is_exact_rust_version("nightly"));
        assert!(is_exact_rust_version("1.85.1"));
        assert!(is_exact_rust_version("nightly-2026-08-14"));
    }

    #[test]
    fn accepts_exact_cargo_prerelease_versions() {
        assert!(is_exact_cargo_version("1.2.3-beta.1"));
        assert!(!is_exact_cargo_version("latest"));
    }
}
