use crate::environment::{
    PythonVersion, exact_python_version, parse_python_release, python_requirement_satisfied,
};
use ayni_core::{
    AdapterError, EnvironmentResolutionCapability, EnvironmentResolutionRequest, Language,
    TargetEnvironment, VersionRequirement,
};
use std::time::Duration;

#[derive(Debug, Default)]
pub(crate) struct PythonEnvironmentResolutionCapability;

impl EnvironmentResolutionCapability for PythonEnvironmentResolutionCapability {
    fn language(&self) -> Language {
        Language::Python
    }

    fn resolve(
        &self,
        request: &EnvironmentResolutionRequest,
    ) -> Result<TargetEnvironment, AdapterError> {
        let mut target = request.target().clone();
        for runtime in &mut target.runtimes {
            runtime.version = resolve_python(request, &runtime.version)?;
        }
        if let Some(manager) = &mut target.package_manager {
            manager.version = resolve_latest(request, "uv", &manager.version)?;
        }
        Ok(target)
    }
}

fn resolve_python(
    request: &EnvironmentResolutionRequest,
    requirement: &VersionRequirement,
) -> Result<VersionRequirement, AdapterError> {
    if requirement.is_exact() {
        return Ok(requirement.clone());
    }
    let expression = requirement_expression("python", requirement)?;
    let output = run_mise(request, "python", &["ls-remote".into(), "python".into()])?;
    let version = output
        .lines()
        .filter_map(|line| {
            let value = line.trim().trim_start_matches("python-");
            exact_python_version(value)
                .ok()
                .flatten()
                .map(|parsed| (parsed, value.to_owned()))
        })
        .filter(|(version, _)| python_selector_satisfied(*version, expression).unwrap_or(false))
        .max_by_key(|(version, _)| *version)
        .map(|(_, value)| value)
        .ok_or_else(|| {
            err(format!(
                "mise returned no exact Python version matching {expression}"
            ))
        })?;
    VersionRequirement::exact(version).map_err(err)
}

fn python_selector_satisfied(
    version: PythonVersion,
    expression: &str,
) -> Result<bool, AdapterError> {
    if expression
        .bytes()
        .all(|byte| byte.is_ascii_digit() || byte == b'.')
    {
        let selected = parse_python_release(expression)?;
        return Ok(version[0] == selected[0]
            && version[1] == selected[1]
            && (expression.split('.').count() < 3 || version[2] == selected[2]));
    }
    python_requirement_satisfied(version, expression)
}

fn resolve_latest(
    request: &EnvironmentResolutionRequest,
    name: &str,
    requirement: &VersionRequirement,
) -> Result<VersionRequirement, AdapterError> {
    if requirement.is_exact() {
        return Ok(requirement.clone());
    }
    let expression = requirement_expression(name, requirement)?;
    let query = format!("{name}@{expression}");
    let output = run_mise(request, name, &["latest".into(), query.clone()])?;
    let value = output.trim();
    if !is_exact_numeric_version(value) {
        return Err(err(format!(
            "mise did not return an exact version for {query}: {value}"
        )));
    }
    VersionRequirement::exact(value).map_err(err)
}

fn requirement_expression<'a>(
    name: &str,
    requirement: &'a VersionRequirement,
) -> Result<&'a str, AdapterError> {
    match requirement {
        VersionRequirement::Selector { expression }
        | VersionRequirement::Compatibility { expression } => Ok(expression),
        VersionRequirement::Minimum { version } => Ok(version),
        VersionRequirement::Unresolved { reason } => {
            Err(err(format!("cannot resolve {name}: {reason}")))
        }
        VersionRequirement::Exact { version } => Ok(version),
    }
}

fn run_mise(
    request: &EnvironmentResolutionRequest,
    name: &str,
    command: &[String],
) -> Result<String, AdapterError> {
    let mut args = vec!["--no-config".into(), "--no-env".into(), "--no-hooks".into()];
    args.extend_from_slice(command);
    let output = ayni_adapters_common::exec::run_command(
        request.repo_root(),
        "mise",
        &args,
        Duration::from_secs(120),
    )
    .map_err(|cause| {
        AdapterError::execution(
            Language::Python,
            format!("failed to run mise for {name}: {cause}"),
        )
    })?;
    if !output.status.success() {
        return Err(AdapterError::execution(
            Language::Python,
            format!("mise could not resolve {name}"),
        ));
    }
    String::from_utf8(output.stdout).map_err(|cause| {
        AdapterError::execution(
            Language::Python,
            format!("mise returned non-UTF-8 output for {name}: {cause}"),
        )
    })
}

fn is_exact_numeric_version(value: &str) -> bool {
    let parts = value.split('.').collect::<Vec<_>>();
    parts.len() == 3
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

fn err(message: impl std::fmt::Display) -> AdapterError {
    AdapterError::new(Language::Python, message.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_versions_and_python_ranges_are_strict() {
        assert!(is_exact_numeric_version("3.12.4"));
        assert!(!is_exact_numeric_version("3.12"));
        assert!(!is_exact_numeric_version("latest"));
        assert!(python_selector_satisfied([3, 12, 4], ">=3.12,<3.13").expect("range"));
        assert!(!python_selector_satisfied([3, 13, 0], "~=3.12.1").expect("range"));
        assert!(python_selector_satisfied([3, 12, 9], "3.12").expect("selector"));
    }
}
