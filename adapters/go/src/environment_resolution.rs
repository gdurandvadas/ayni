use ayni_core::{
    AdapterError, EnvironmentResolutionCapability, EnvironmentResolutionRequest, Language,
    TargetEnvironment, VersionRequirement,
};
use std::time::Duration;

#[derive(Debug, Default)]
pub(crate) struct GoEnvironmentResolutionCapability;

impl EnvironmentResolutionCapability for GoEnvironmentResolutionCapability {
    fn language(&self) -> Language {
        Language::Go
    }

    fn resolve(
        &self,
        request: &EnvironmentResolutionRequest,
    ) -> Result<TargetEnvironment, AdapterError> {
        let mut target = request.target().clone();
        for runtime in &mut target.runtimes {
            runtime.version = resolve_mise(request, "go", &runtime.version, "go")?;
        }
        for tool in &mut target.signal_tools {
            if !tool.provider.starts_with("go:") {
                return Err(error(format!(
                    "unsupported Go signal-tool provider {} for {}",
                    tool.provider, tool.tool
                )));
            }
            tool.version = resolve_mise(request, &tool.tool, &tool.version, &tool.provider)?;
        }
        Ok(target)
    }
}

fn resolve_mise(
    request: &EnvironmentResolutionRequest,
    name: &str,
    requirement: &VersionRequirement,
    provider: &str,
) -> Result<VersionRequirement, AdapterError> {
    if requirement.is_exact() {
        return Ok(requirement.clone());
    }
    let query = match requirement {
        VersionRequirement::Selector { expression }
        | VersionRequirement::Compatibility { expression } => format!("{provider}@{expression}"),
        VersionRequirement::Minimum { version } => format!("{provider}@{version}"),
        VersionRequirement::Unresolved { .. } => provider.to_owned(),
        VersionRequirement::Exact { .. } => unreachable!(),
    };
    let args = vec![
        "--no-config".into(),
        "--no-env".into(),
        "--no-hooks".into(),
        "latest".into(),
        query.clone(),
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
    let version = version.trim().trim_start_matches('v');
    if !is_exact_go_version(version) {
        return Err(error(format!(
            "mise did not return an exact version for {query}: {version}"
        )));
    }
    VersionRequirement::exact(version).map_err(|cause| error(cause.to_string()))
}

fn is_exact_go_version(value: &str) -> bool {
    value.split('.').count() == 3
        && value
            .split('.')
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}
fn provider_error(message: impl Into<String>) -> AdapterError {
    AdapterError::execution(Language::Go, message)
}
fn error(message: impl Into<String>) -> AdapterError {
    AdapterError::new(Language::Go, message)
}

#[cfg(test)]
mod tests {
    use super::is_exact_go_version;
    #[test]
    fn accepts_only_exact_go_versions() {
        assert!(is_exact_go_version("1.24.3"));
        assert!(!is_exact_go_version("1.24"));
        assert!(!is_exact_go_version("latest"));
        assert!(!is_exact_go_version("go1.24.3"));
    }
}
